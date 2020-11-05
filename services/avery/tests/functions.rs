use slog::o;

use avery::{executor::ExecutionService, registry::RegistryService};

use firm_types::{
    execution::{execution_server::Execution, ExecutionParameters},
    functions::{ChannelSpec, ChannelType},
    registry::registry_server::Registry,
    stream::ToChannel,
    tonic,
};

use firm_types::{channel_specs, filters, function_data, runtime, stream};

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
        ExecutionService::new(null_logger!(), Box::new($service.clone()))
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
        RegistryService::new(null_logger!())
    }};
}

#[test]
fn test_execute() {
    let sr = registry_service!();
    let svc = register_functions!(
        sr,
        vec![function_data!(
            "say-hello-yourself",
            "0.1.0",
            runtime!("wasi"),
            register_code_attachment!(sr).id,
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
            channel_specs!(
                {
                    "output_string" => ChannelSpec {
                        description: "yes".to_owned(),
                        r#type: ChannelType::String as i32,
                    }
                }
            )
            .0,
            [], // attachments
            {}  // metadata
        )]
    );

    let correct_args = stream!({ "say" => "sune", "count" => 7i64 });
    let ff = first_function!(sr);
    let r = futures::executor::block_on(svc.execute_function(tonic::Request::new(
        ExecutionParameters {
            name: ff.name,
            version_requirement: ff.version,
            arguments: Some(correct_args),
        },
    )));
    assert!(r.is_ok());

    let incorrect_args = stream!({ "say" => 7, "count" => "nope" });

    let ff = first_function!(sr);
    let r = futures::executor::block_on(svc.execute_function(tonic::Request::new(
        ExecutionParameters {
            name: ff.name,
            version_requirement: ff.version,
            arguments: Some(incorrect_args),
        },
    )));
    assert!(r.is_err());
}
