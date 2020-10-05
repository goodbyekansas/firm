use quinn::storage;
use slog::{o, Drain, Logger};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let log = Logger::root(drain, o!());

    let config = quinn::config::Configuration::new(log.clone()).await?;
    let storage = storage::create_storage(config.functions_storage_uri, log).await?;
    storage
        .insert(storage::FunctionData {
            name: "Hästsko".into(),
            version: semver::Version::new(1, 9999, 2),
            execution_environment: storage::ExecutionEnvironment {
                name: "¥".to_string(),
                entrypoint: "in här".to_owned(),
                function_arguments: HashMap::new(),
            },
            inputs: vec![],
            outputs: vec![],
            metadata: HashMap::new(),
            code: None,
            attachments: vec![],
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
        .await?;
    storage
        .insert(storage::FunctionData {
            name: "attached-hästsko🏇".into(),
            version: semver::Version::new(1, 9999, 2),
            execution_environment: storage::ExecutionEnvironment {
                name: "¥".to_string(),
                entrypoint: "in här".to_owned(),
                function_arguments: HashMap::new(),
            },
            inputs: vec![],
            outputs: vec![],
            metadata: HashMap::new(),
            code: None,
            attachments: vec![attachment_id],
        })
        .await?;
    storage
        .insert(storage::FunctionData {
            name: "samvetskval".into(),
            version: semver::Version::parse("1.666.1-bra")?,
            execution_environment: storage::ExecutionEnvironment {
                name: "¥".to_string(),
                entrypoint: "in här".to_owned(),
                function_arguments: HashMap::new(),
            },
            inputs: vec![],
            outputs: vec![],
            metadata: HashMap::new(),
            code: None,
            attachments: vec![],
        })
        .await?;
    storage
        .insert(storage::FunctionData {
            name: "samvetskval".into(),
            version: semver::Version::parse("1.666.1")?,
            execution_environment: storage::ExecutionEnvironment {
                name: "¥".to_string(),
                entrypoint: "in här".to_owned(),
                function_arguments: HashMap::new(),
            },
            inputs: vec![],
            outputs: vec![],
            metadata: HashMap::new(),
            code: None,
            attachments: vec![],
        })
        .await?;
    Ok(())
}
