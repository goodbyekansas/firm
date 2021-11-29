use quinn::storage;
use slog::{o, Drain, Logger};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let log = Logger::root(drain, o!());

    let config = quinn::config::Configuration::new_with_init(log.clone(), |c| {
        c.set(
            "attachment_storage_uri",
            "https://false.com/no-attachments/",
        )
    })
    .await?;
    let storage = storage::create_storage(config.functions_storage_uri, log).await?;
    storage
        .insert(storage::Function {
            name: "Hästsko".into(),
            version: semver::Version::new(1, 9999, 2),
            runtime: storage::Runtime {
                name: "¥".to_string(),
                entrypoint: "in här".to_owned(),
                arguments: HashMap::new(),
            },
            required_inputs: HashMap::new(),
            optional_inputs: HashMap::new(),
            outputs: HashMap::new(),
            metadata: HashMap::new(),
            code: None,
            attachments: vec![],
            created_at: 4,
            publisher: storage::Publisher {
                name: String::from("Johansen Rackstr"),
                email: String::from("l1333@rackstr.no"),
            },
            signature: None,
        })
        .await?;
    let attachment_id = storage
        .insert_attachment(storage::FunctionAttachmentData {
            name: "attackment!".to_string(),
            metadata: HashMap::new(),
            checksums: storage::Checksums {
                sha256: "🚢🛥️⛴️🚤".to_owned(),
            },
            publisher: storage::Publisher {
                name: "Skrubb Skädda".to_owned(),
                email: "skrubb.skadda@fisk.se".to_owned(),
            },
            signature: None,
        })
        .await?
        .id;
    storage
        .insert(storage::Function {
            name: "attached-hästsko🏇".into(),
            version: semver::Version::new(1, 9999, 2),
            runtime: storage::Runtime {
                name: "¥".to_string(),
                entrypoint: "in här".to_owned(),
                arguments: HashMap::new(),
            },
            required_inputs: HashMap::new(),
            optional_inputs: HashMap::new(),
            outputs: HashMap::new(),
            metadata: HashMap::new(),
            code: None,
            attachments: vec![attachment_id],
            created_at: 3,
            publisher: storage::Publisher {
                name: String::from("Budas"),
                email: String::from("budas@budarsson.com"),
            },
            signature: None,
        })
        .await?;
    storage
        .insert(storage::Function {
            name: "samvetskval".into(),
            version: semver::Version::parse("1.666.1-bra")?,
            runtime: storage::Runtime {
                name: "¥".to_string(),
                entrypoint: "in här".to_owned(),
                arguments: HashMap::new(),
            },
            required_inputs: HashMap::new(),
            optional_inputs: HashMap::new(),
            outputs: HashMap::new(),
            metadata: HashMap::new(),
            code: None,
            attachments: vec![],
            created_at: 2,
            publisher: storage::Publisher {
                name: String::from("sven-benny"),
                email: String::from("sven-benny@svenbennysson.com"),
            },
            signature: None,
        })
        .await?;
    storage
        .insert(storage::Function {
            name: "samvetskval".into(),
            version: semver::Version::parse("1.666.1")?,
            runtime: storage::Runtime {
                name: "¥".to_string(),
                entrypoint: "in här".to_owned(),
                arguments: HashMap::new(),
            },
            required_inputs: HashMap::new(),
            optional_inputs: HashMap::new(),
            outputs: HashMap::new(),
            metadata: HashMap::new(),
            code: None,
            attachments: vec![],
            created_at: 1,
            publisher: storage::Publisher {
                name: String::from("Lasse-gurra Aktersnurra"),
                email: String::from("lassegurra@aktersnurra.se"),
            },
            signature: None,
        })
        .await?;
    Ok(())
}
