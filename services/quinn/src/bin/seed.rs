use quinn::storage;
use slog::{o, Drain, Logger};
use std::collections::HashMap;
use storage::StreamSpec;

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
            input_spec: StreamSpec::empty(),
            output_spec: StreamSpec::empty(),
            metadata: HashMap::new(),
            code: None,
            attachments: vec![],
            created_at: 4,
        })
        .await?;
    let attachment_id = storage
        .insert_attachment(storage::FunctionAttachmentData {
            name: "attackment!".to_string(),
            metadata: HashMap::new(),
            checksums: storage::Checksums {
                sha256: "🚢🛥️⛴️🚤".to_owned(),
            },
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
            input_spec: StreamSpec::empty(),
            output_spec: StreamSpec::empty(),
            metadata: HashMap::new(),
            code: None,
            attachments: vec![attachment_id],
            created_at: 3,
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
            input_spec: StreamSpec::empty(),
            output_spec: StreamSpec::empty(),
            metadata: HashMap::new(),
            code: None,
            attachments: vec![],
            created_at: 2,
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
            input_spec: StreamSpec::empty(),
            output_spec: StreamSpec::empty(),
            metadata: HashMap::new(),
            code: None,
            attachments: vec![],
            created_at: 1,
        })
        .await?;
    Ok(())
}
