use std::{
    fmt::Debug,
    fs::OpenOptions,
    io::{BufReader, Write},
    path::{Path, PathBuf},
    str::FromStr,
    sync::RwLock,
    time::SystemTime,
};

use rustls::{
    client::{ServerCertVerifier, WebPkiVerifier},
    Certificate, RootCertStore, ServerName,
};
use sha1::{Digest, Sha1};

#[derive(Debug, PartialEq)]
pub enum SelfSignedAnswer {
    Yes,
    No,
    Save,
}

impl FromStr for SelfSignedAnswer {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "y" | "yes" => Ok(SelfSignedAnswer::Yes),
            "n" | "no" => Ok(SelfSignedAnswer::No),
            "s" | "save" => Ok(SelfSignedAnswer::Save),
            _ => Err(format!(
                "Could not convert \"{}\" to the SelfSignedAnswer enum.",
                s
            )),
        }
    }
}

//                           host, alt_names, fingerprint
type SelfSignedCallback = fn(&str, &str, &str) -> Result<SelfSignedAnswer, String>;

pub struct CertVerifier {
    cert_bundle_path: PathBuf,
    root_store: RwLock<RootCertStore>,
    self_signed_cb: Option<SelfSignedCallback>,
}

impl Debug for CertVerifier {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        writeln!(
            fmt,
            "InteractiveCertVerifier saving accepted certs @ {}",
            self.cert_bundle_path.display()
        )
    }
}

impl CertVerifier {
    pub fn try_new(
        root_directory: &Path,
        certs: &[Certificate],
        self_signed_cb: Option<SelfSignedCallback>,
    ) -> Result<Self, std::io::Error> {
        let cert_bundle_path = root_directory.join("cert-bundle.pem");
        let mut root_store = RootCertStore::empty();

        // add provided certificates
        certs
            .iter()
            .try_for_each(|cert| root_store.add(cert))
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::Other))?;

        // parse certificates from bundle file
        // we store both the full x509 certificate
        // and the webpki subset needed for validation. This is
        // since we want to be able to write out a complete PEM cert
        // chain again
        if cert_bundle_path.exists() {
            root_store.add_parsable_certificates(
                &std::fs::OpenOptions::new()
                    .read(true)
                    .open(&cert_bundle_path)
                    .map(BufReader::new)
                    .and_then(|mut rdr| {
                        rustls_pemfile::certs(&mut rdr)
                            .map_err(|_| std::io::ErrorKind::InvalidData.into())
                    })?,
            );
        }

        Ok(Self {
            root_store: RwLock::new(root_store),
            cert_bundle_path,
            self_signed_cb,
        })
    }

    fn get_alt_names_and_fingerprint(
        end_entity: &Certificate,
    ) -> Result<(String, String), rustls::Error> {
        x509_parser::parse_x509_certificate(end_entity.as_ref())
            .map_err(|e| rustls::Error::General(format!("Failed to parse x509 certificate: {}", e)))
            .map(|(_, parsed)| (Sha1::digest(end_entity.as_ref()), parsed))
            .and_then(|(fingerprint, parsed)| {
                parsed
                    .extensions()
                    .get(&der_parser::oid!(2.5.29 .17))
                    .ok_or_else(|| {
                        rustls::Error::General(
                            "Certificate does not contain SubjectAlternativeName extension"
                                .to_owned(),
                        )
                    })
                    .and_then(|alt_names_ext| match alt_names_ext.parsed_extension() {
                        x509_parser::extensions::ParsedExtension::SubjectAlternativeName(names) => {
                            Ok(names
                                .general_names
                                .iter()
                                .map(|n| match n {
                                    x509_parser::extensions::GeneralName::OtherName(_, _) => {
                                        "other".to_owned()
                                    }
                                    x509_parser::extensions::GeneralName::RFC822Name(n) => {
                                        n.to_string()
                                    }
                                    x509_parser::extensions::GeneralName::DNSName(n) => {
                                        n.to_string()
                                    }
                                    x509_parser::extensions::GeneralName::DirectoryName(n) => {
                                        n.to_string()
                                    }
                                    x509_parser::extensions::GeneralName::URI(_) => {
                                        "uri".to_owned()
                                    }
                                    x509_parser::extensions::GeneralName::IPAddress(_) => {
                                        "ip".to_owned()
                                    }
                                    x509_parser::extensions::GeneralName::RegisteredID(_) => {
                                        "id".to_owned()
                                    }
                                })
                                .collect::<Vec<String>>()
                                .join(", "))
                        }
                        _ => Err(rustls::Error::General(
                            "SubjectAlternativeName extension has the wrong type!?".to_owned(),
                        )),
                    })
                    .map(|alt_names| {
                        (
                            format!("{:X}", fingerprint)
                                .chars()
                                .enumerate()
                                .flat_map(|(i, c)| {
                                    if i != 0 && i % 2 == 0 {
                                        Some(':')
                                    } else {
                                        None
                                    }
                                    .into_iter()
                                    .chain(std::iter::once(c))
                                })
                                .collect::<String>(),
                            alt_names,
                        )
                    })
            })
    }

    fn add_certs_to_root(
        &self,
        end_entity: &Certificate,
        intermediates: &[Certificate],
    ) -> Result<(), rustls::Error> {
        let mut store = self.root_store.write().map_err(|e| {
            rustls::Error::General(format!(
                "Failed to acquire write lock to add cert to root: {}",
                e
            ))
        })?;

        store
            .add(end_entity)
            .map_err(|e| rustls::Error::General(e.to_string()))?;
        intermediates
            .iter()
            .try_for_each(|cert| store.add(cert))
            .map_err(|e| rustls::Error::General(e.to_string()))
    }

    fn save_certificate_chain(
        &self,
        end_entity: &Certificate,
        intermediates: &[Certificate],
    ) -> Result<(), rustls::Error> {
        OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.cert_bundle_path)
            .map_err(|e| {
                rustls::Error::General(format!(
                    "Failed to open cert bundle file at {}: {}",
                    self.cert_bundle_path.display(),
                    e
                ))
            })
            .and_then(|mut f| {
                f.write(
                    pem::encode_many(
                        std::iter::once(end_entity)
                            .chain(intermediates)
                            .map(|c| pem::Pem {
                                tag: String::from("CERTIFICATE"),
                                contents: c.as_ref().to_vec(),
                            })
                            .collect::<Vec<pem::Pem>>()
                            .as_slice(),
                    )
                    .as_bytes(),
                )
                .map(|_| ())
                .map_err(|e| {
                    rustls::Error::General(format!(
                        "Failed to write cert bundle file at {}: {}",
                        self.cert_bundle_path.display(),
                        e
                    ))
                })
            })
    }
}

impl ServerCertVerifier for CertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &Certificate,
        intermediates: &[Certificate],
        server_name: &ServerName,
        scts: &mut dyn Iterator<Item = &[u8]>,
        ocsp_response: &[u8],
        now: SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        self.root_store
            .read()
            .map_err(|e| {
                rustls::Error::General(format!(
                    "Failed to acquire read lock for root cert store: {}",
                    e
                ))
            })
            .and_then(|roots| {
                WebPkiVerifier::new(roots.clone(), None).verify_server_cert(
                    end_entity,
                    intermediates,
                    server_name,
                    scts,
                    ocsp_response,
                    now,
                )
            })
            .or_else(|e| {
                if let rustls::Error::InvalidCertificateSignature = e {
                    let (fingerprint, alt_names) = Self::get_alt_names_and_fingerprint(end_entity)
                        .unwrap_or_else(|e| (String::new(), format!("<unknown: {}>", e)));
                    let host = match server_name {
                        ServerName::DnsName(name) => name.as_ref().to_string(),
                        ServerName::IpAddress(address) => address.to_string(),
                        _ => String::from("unknown"),
                    };
                    let answer = match self.self_signed_cb.as_ref() {
                        Some(cb) => (cb)(&host, &alt_names, &fingerprint).map_err(|e| {
                            rustls::Error::General(format!("Self signed callback error: {}", e))
                        }),
                        None => Ok(SelfSignedAnswer::No),
                    }?;

                    if answer == SelfSignedAnswer::Yes || answer == SelfSignedAnswer::Save {
                        self.add_certs_to_root(end_entity, intermediates)?;
                        self.root_store
                            .read()
                            .map_err(|e| {
                                rustls::Error::General(format!(
                                    "Failed to acquire read lock for root cert store: {}",
                                    e
                                ))
                            })
                            .and_then(|roots| {
                                WebPkiVerifier::new(roots.clone(), None).verify_server_cert(
                                    end_entity,
                                    intermediates,
                                    server_name,
                                    scts,
                                    ocsp_response,
                                    now,
                                )
                            })
                            .and_then(|r| {
                                if answer == SelfSignedAnswer::Save {
                                    self.save_certificate_chain(end_entity, intermediates)?;
                                }
                                Ok(r)
                            })
                    } else {
                        Err(e)
                    }
                } else {
                    Err(e)
                }
            })
    }
}
