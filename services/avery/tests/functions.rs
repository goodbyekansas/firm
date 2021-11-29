use std::{ops::Deref, thread};

use futures::StreamExt;
use slog::o;

use avery::{
    auth::AuthService, config::InternalRegistryConfig, executor::ExecutionService,
    registry::RegistryService,
};

use firm_types::{
    functions::FunctionOutputChunk,
    functions::{
        execution_server::Execution, registry_server::Registry, AttachmentStreamUpload,
        ChannelSpec, ChannelType, ExecutionParameters,
    },
    stream::ToChannel,
    tonic,
};

use firm_types::{channel_specs, filters, function_data, runtime_spec, stream};

macro_rules! null_logger {
    () => {{
        slog::Logger::root(slog::Discard, o!())
    }};
}

macro_rules! register_code_attachment {
    ($service:expr) => {{
        futures::executor::block_on(
            $service.register_attachment(tonic::Request::new(firm_types::attachment_data!("code"))),
        )
        .unwrap()
        .into_inner()
    }};
    ($service:expr, $content:expr, $sha256:expr) => {{
        let id = futures::executor::block_on($service.register_attachment(tonic::Request::new(
            firm_types::attachment_data!("code", $sha256),
        )))
        .unwrap()
        .into_inner();
        let code = Ok(AttachmentStreamUpload {
            id: id.id.clone(),
            content: $content,
        });
        futures::executor::block_on(
            $service
                .upload_stream_attachment(tonic::Request::new(futures::stream::iter(vec![code]))),
        )
        .unwrap();
        id
    }};
}
struct ExecutionServiceWrapper {
    execution_service: ExecutionService,
    _temp_root_dir: tempfile::TempDir,
}

impl ExecutionServiceWrapper {
    fn new(execution_service: ExecutionService, temp_root_dir: tempfile::TempDir) -> Self {
        Self {
            execution_service,
            _temp_root_dir: temp_root_dir,
        }
    }
}

impl Deref for ExecutionServiceWrapper {
    type Target = ExecutionService;

    fn deref(&self) -> &Self::Target {
        &self.execution_service
    }
}

macro_rules! register_functions {
    ($service:expr, $fns:expr) => {{
        $fns.into_iter().for_each(|f| {
            futures::executor::block_on($service.register(tonic::Request::new(f.clone())))
                .map_or_else(
                    |e| println!("Failed to register function \"{}\". Err: {}", f.name, e),
                    |_| (),
                );
        });
        let temp_root_directory = tempfile::TempDir::new().unwrap();
        ExecutionServiceWrapper::new(
            ExecutionService::new(
                null_logger!(),
                $service.clone(),
                vec![Box::new(avery::runtime::InternalRuntimeSource::new(
                    null_logger!(),
                ))],
                AuthService::default(),
                temp_root_directory.path(),
            )
            .unwrap(),
            temp_root_directory,
        )
    }};
}

macro_rules! first_function {
    ($service:expr) => {{
        futures::executor::block_on($service.list(tonic::Request::new(filters!())))
            .unwrap()
            .into_inner()
            .functions
            .first()
            .unwrap()
            .clone()
    }};
}

macro_rules! registry_service {
    () => {{
        RegistryService::new(InternalRegistryConfig::default(), null_logger!()).unwrap()
    }};
}

#[tokio::test]
async fn execute() {
    let registry_service = registry_service!();
    let execution_service = register_functions!(
        registry_service,
        vec![function_data!(
            "say-hello-yourself",
            "0.1.0",
            runtime_spec!("wasi"),
            register_code_attachment!(
                registry_service,
                include_bytes!("../src/runtime/hello.wasm").to_vec(),
                "c455c4bc68c1afcdafa7c2f74a499810b0aa5d12f7a009d493789d595847af72"
            )
            .id,
            channel_specs!(
                {
                    "say" => ChannelSpec {
                        description: "no".to_owned(),
                        r#type: ChannelType::String as i32,
                    },
                    "count" => ChannelSpec {
                        description: "yes".to_owned(),
                        r#type: ChannelType::Int as i32,
                    }
                }
            )
            .0,
            std::collections::HashMap::new(),
            channel_specs!({}).0,
            "Publisher",
            "publisher@company.com",
            [], // attachments
            {}  // metadata
        )]
    );

    let ff = first_function!(registry_service);
    let correct_args = stream!({ "say" => "sune", "count" => 7i64 });

    // Test without reading output
    let r = futures::executor::block_on(execution_service.queue_function(tonic::Request::new(
        ExecutionParameters {
            name: ff.name.clone(),
            version_requirement: ff.version.clone(),
            arguments: Some(correct_args.clone()),
        },
    )));
    assert!(r.is_ok());
    let eid = r.unwrap().into_inner();

    let r = futures::executor::block_on(execution_service.run_function(tonic::Request::new(eid)));
    assert!(r.is_ok());

    // Test checking for correct args and output is getting propagated
    let r = futures::executor::block_on(execution_service.queue_function(tonic::Request::new(
        ExecutionParameters {
            name: ff.name.clone(),
            version_requirement: ff.version.clone(),
            arguments: Some(correct_args),
        },
    )));
    assert!(r.is_ok());
    let eid = r.unwrap().into_inner();

    let stream = futures::executor::block_on(
        execution_service.function_output(tonic::Request::new(eid.clone())),
    )
    .unwrap()
    .into_inner();

    let t = thread::spawn(move || {
        let chunks: Vec<Result<FunctionOutputChunk, tonic::Status>> =
            futures::executor::block_on(async move { stream.collect().await });
        assert!(chunks.iter().all(|cr| {
            match cr {
                Ok(c) => c.channel == "stdout",
                Err(_) => false,
            }
        }));
        assert_eq!(
            chunks.iter().fold(String::new(), |mut acc, cur| {
                acc.push_str(cur.as_ref().map_or("", |s| s.output.as_str()));
                acc
            }),
            "hello world\n"
        );
    });

    let r = futures::executor::block_on(execution_service.run_function(tonic::Request::new(eid)));

    assert!(r.is_ok());
    t.join().unwrap();

    // Test incorrect args
    let incorrect_args = stream!({ "say" => 7, "count" => "nope" });
    let r = futures::executor::block_on(execution_service.queue_function(tonic::Request::new(
        ExecutionParameters {
            name: ff.name,
            version_requirement: ff.version,
            arguments: Some(incorrect_args),
        },
    )));
    assert!(r.is_err());
}
