use std::{
    fmt::Debug,
    fs::OpenOptions,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::RwLock,
};

use rustls::{Certificate, RootCertStore, ServerCertVerifier, WebPKIVerifier};
use sha1::{Digest, Sha1};

pub struct InteractiveCertVerifier {
    inner: WebPKIVerifier,
    cert_bundle: PathBuf,
    root_store: RwLock<RootCertStore>,
    root_store_certs: RwLock<Vec<Certificate>>,
}

impl Debug for InteractiveCertVerifier {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        writeln!(
            fmt,
            "InteractiveCertVerifier saving accepted certs @ {}",
            self.cert_bundle.display()
        )
    }
}

impl InteractiveCertVerifier {
    pub fn new(root_directory: &Path) -> Result<Self, std::io::Error> {
        let mut icv = Self {
            inner: WebPKIVerifier::new(),
            cert_bundle: root_directory.join("cert-bundle.pem"),
            root_store: RwLock::new(RootCertStore::empty()),
            root_store_certs: RwLock::new(Vec::new()),
        };

        if icv.cert_bundle.exists() {
            // parse certificates from bundle file
            // we store both the full x509 certificate
            // and the webpki subset needed for validation. This is
            // since we want to be able to write out a complete PEM cert
            // chain again
            let root_store_certs = std::fs::OpenOptions::new()
                .read(true)
                .open(&icv.cert_bundle)
                .map(BufReader::new)
                .and_then(|mut rdr| {
                    rustls::internal::pemfile::certs(&mut rdr)
                        .map_err(|_| std::io::ErrorKind::InvalidData.into())
                })?;

            let mut root_store = RootCertStore::empty();
            root_store_certs
                .iter()
                .try_for_each(|cert| root_store.add(cert))
                .map_err(|_| std::io::Error::from(std::io::ErrorKind::Other))?;

            icv.root_store_certs = RwLock::new(root_store_certs);
            icv.root_store = RwLock::new(root_store);
        }

        Ok(icv)
    }

    fn get_alt_names_and_fingerprint(
        presented_certs: &[Certificate],
    ) -> Result<(String, String), rustls::TLSError> {
        presented_certs
            .first()
            .ok_or(rustls::TLSError::NoCertificatesPresented)
            .and_then(|c| {
                x509_parser::parse_x509_certificate(c.as_ref())
                    .map_err(|e| {
                        rustls::TLSError::General(format!(
                            "Failed to parse x509 certificate: {}",
                            e
                        ))
                    })
                    .map(|(_, parsed)| (Sha1::digest(c.as_ref()), parsed))
            })
            .and_then(|(fingerprint, parsed)| {
                parsed
                    .extensions()
                    .get(&der_parser::oid!(2.5.29 .17))
                    .ok_or_else(|| {
                        rustls::TLSError::General(
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
                        _ => Err(rustls::TLSError::General(
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

    fn add_certs_to_root(&self, presented_certs: &[Certificate]) -> Result<(), rustls::TLSError> {
        self.root_store_certs
            .write()
            .map_err(|e| {
                rustls::TLSError::General(format!(
                    "Failed to acquire write lock to add cert to root: {}",
                    e
                ))
            })?
            .extend_from_slice(presented_certs);

        self.root_store
            .try_write()
            .map_err(|e| {
                rustls::TLSError::General(format!(
                    "Failed to acquire write lock to add cert to root store: {}",
                    e
                ))
            })
            .and_then(|mut root_store| {
                presented_certs
                    .iter()
                    .try_for_each(|c| root_store.add(c).map_err(rustls::TLSError::WebPKIError))
            })
    }

    fn merge_root_from(&self, root_store: &RootCertStore) -> Result<(), rustls::TLSError> {
        // note that since these are not added to `root_store_certs`
        // these certs will not (should not and can not) be saved to disk.
        self.root_store
            .write()
            .map_err(|e| {
                rustls::TLSError::General(format!(
                    "Failed to acquire write lock to add cert to root store: {}",
                    e
                ))
            })
            .map(|mut store| store.roots.extend_from_slice(&root_store.roots))
    }

    fn save_roots(&self) -> Result<(), rustls::TLSError> {
        self.root_store_certs
            .read()
            .map_err(|e| {
                rustls::TLSError::General(format!(
                    "Failed to acquire read lock to save certs to file: {}",
                    e
                ))
            })
            .and_then(|root_store_certs| {
                OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&self.cert_bundle)
                    .map_err(|e| {
                        rustls::TLSError::General(format!(
                            "Failed to open cert bundle file at {}: {}",
                            self.cert_bundle.display(),
                            e
                        ))
                    })
                    .and_then(|mut f| {
                        f.write(
                            pem::encode_many(
                                root_store_certs
                                    .iter()
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
                            rustls::TLSError::General(format!(
                                "Failed to write cert bundle file at {}: {}",
                                self.cert_bundle.display(),
                                e
                            ))
                        })
                    })
            })
    }
}

impl ServerCertVerifier for InteractiveCertVerifier {
    fn verify_server_cert(
        &self,
        roots: &RootCertStore,
        presented_certs: &[Certificate],
        dns_name: webpki::DNSNameRef,
        ocsp_response: &[u8],
    ) -> std::result::Result<rustls::ServerCertVerified, rustls::TLSError> {
        self.merge_root_from(roots)?;
        self.root_store
            .read()
            .map_err(|e| {
                rustls::TLSError::General(format!(
                    "Failed to acquire read lock for root cert store: {}",
                    e
                ))
            })
            .and_then(|roots| {
                self.inner
                    .verify_server_cert(&roots, presented_certs, dns_name, ocsp_response)
            })
            .or_else(|e| {
                if let rustls::TLSError::WebPKIError(webpki::Error::UnknownIssuer) = e {
                    let (fingerprint, alt_names) =
                        Self::get_alt_names_and_fingerprint(presented_certs)
                            .unwrap_or_else(|e| (String::new(), format!("<unknown: {}>", e)));

                    println!(
                        "{}",
                        warn!(
                            "Host \"{}\" is using a self-signed certificate.",
                            AsRef::<str>::as_ref(&dns_name.to_owned())
                        )
                    );
                    println!(
                        "The host identifies as \"{}\" [{}].",
                        // An error to parse alt name extension from the cert is
                        // not really critical but it will hopefully look intimidating
                        // enough to the user that they will do the right thing
                        ansi_term::Style::new().bold().paint(alt_names),
                        fingerprint
                    );
                    print!("Do you want to continue? [y(es)/n(o)/S(ave)] ");
                    let _ = std::io::stdout().flush(); // don't really care if we fail to flush

                    // read the user answer and assume "no" if we for
                    // some reason cannot read the answer
                    let stdin = std::io::stdin();
                    let line = stdin
                        .lock()
                        .lines()
                        .next()
                        .unwrap_or_else(|| Ok("n".to_owned()))
                        .unwrap_or_else(|_| "n".to_owned());

                    if line.to_lowercase() == "s"
                        || line.to_lowercase() == "save"
                        || line.is_empty()
                    {
                        self.add_certs_to_root(presented_certs)?;
                        self.root_store
                            .read()
                            .map_err(|e| {
                                rustls::TLSError::General(format!(
                                    "Failed to acquire read lock for root cert store: {}",
                                    e
                                ))
                            })
                            .and_then(|root| {
                                self.inner
                                    .verify_server_cert(
                                        &root,
                                        presented_certs,
                                        dns_name,
                                        ocsp_response,
                                    )
                                    .and_then(|r| {
                                        self.save_roots()?;
                                        Ok(r)
                                    })
                            })
                    } else if line.to_lowercase() == "y" || line.to_lowercase() == "yes" {
                        self.add_certs_to_root(presented_certs)?;
                        self.root_store
                            .read()
                            .map_err(|e| {
                                rustls::TLSError::General(format!(
                                    "Failed to acquire read lock for root cert store: {}",
                                    e
                                ))
                            })
                            .and_then(|root| {
                                self.inner.verify_server_cert(
                                    &root,
                                    presented_certs,
                                    dns_name,
                                    ocsp_response,
                                )
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
