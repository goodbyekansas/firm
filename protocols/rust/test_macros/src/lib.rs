#[macro_export]
macro_rules! function_attachment {
    () => {{
        $crate::function_attachment!("fake://")
    }};

    ($url:expr) => {{
        $crate::function_attachment!(
            $url,
            "FakeAttachment",
            "724a8940e46ffa34e930258f708d890dbb3b3243361dfbc41eefcff124407a29"
        )
    }};

    ($url:expr, $name:expr) => {{
        $crate::function_attachment!(
            $url,
            $name,
            "724a8940e46ffa34e930258f708d890dbb3b3243361dfbc41eefcff124407a29"
        )
    }};

    ($url:expr, $name:expr, $sha256:expr) => {{
        gbk_protocols::functions::FunctionAttachment {
            name: $name.to_owned(),
            url: $url.to_owned(),
            id: Some(gbk_protocols::functions::FunctionAttachmentId {
                id: uuid::Uuid::new_v4().to_string(),
            }),
            metadata: std::collections::HashMap::new(),
            checksums: Some(gbk_protocols::functions::Checksums {
                sha256: $sha256.to_owned(),
            }),
        }
    }};
}

#[macro_export]
macro_rules! code_file {
    ($content:expr) => {{
        $crate::attachment_file!($content, "code")
    }};
}

#[macro_export]
macro_rules! attachment_file {
    ($content:expr, $name:expr) => {{
        use std::io::Write;
        let tf = tempfile::NamedTempFile::new().unwrap();
        let (mut file, path) = tf.keep().unwrap();
        file.write_all($content).unwrap();

        use sha2::{Digest, Sha256};
        let sha256 = Sha256::digest($content);
        $crate::function_attachment!(
            format!("file://{}", path.display()),
            $name,
            hex::encode(sha256)
        )
    }};
}

#[macro_export]
macro_rules! function_input {
    ($name:expr, $required:expr, $argtype:path) => {{
        $crate::function_input!($name, $required, $argtype, "")
    }};

    ($name:expr, $required:expr, $argtype:path, $default:expr) => {{
        gbk_protocols::functions::FunctionInput {
            name: String::from($name),
            required: $required,
            r#type: $argtype as i32,
            default_value: String::from($default),
            from_execution_environment: false,
        }
    }};
}

#[macro_export]
macro_rules! function_output {
    ($name:expr, $argtype:path) => {{
        gbk_protocols::functions::FunctionOutput {
            name: String::from($name),
            r#type: $argtype as i32,
        }
    }};
}

#[macro_export]
macro_rules! register_request {
    ($name:expr, [$($input:expr),*], [$($output:expr),*], {$($key:expr => $value:expr),*}, $code:expr, $exec_env:expr) => {{
        let mut metadata = ::std::collections::HashMap::new();
        $(
            metadata.insert(String::from($key), String::from($value));
        )*
        gbk_protocols::functions::RegisterRequest {
            name: String::from($name),
            execution_environment: ::std::option::Option::from($exec_env),
            code: ::std::option::Option::from($code),
            version: "0.1.0".to_owned(),
            metadata,
            inputs: vec![$($input),*],
            outputs: vec![$($output),*],
            attachment_ids: vec![],
            host_folder_mounts: vec![],
        }
    }};

    ($name:expr, $version:expr) => {{
        $crate::register_request!($name, $version, exec_env!())
    }};

    ($name:expr, $version:expr, $exe_env:expr) => {{
        $crate::register_request!($name, $version, $exe_env, {})
    }};

    ($name:expr, $version:expr, $exe_env:expr, {$($key:expr => $value:expr),*}) => {{
        $crate::register_request!($name, $version, $exe_env, None, {$($key => $value),*})
    }};

    ($name:expr, $version:expr, $exe_env:expr, $code:expr, {$($key:expr => $value:expr),*}) => {{
        $crate::register_request!($name, $version, $exe_env, $code, [], {$($key => $value),*})
    }};

    ($name:expr, $version:expr, $exe_env:expr, $code:expr, [$($attach:expr),*], {$($key:expr => $value:expr),*}) => {{
        let mut m = ::std::collections::HashMap::new();
        $(
            m.insert(String::from($key), String::from($value));
        )*

        gbk_protocols::functions::RegisterRequest {
            name: String::from($name),
            execution_environment: ::std::option::Option::from($exe_env),
            code: $code,
            version: String::from($version),
            metadata: m,
            inputs: vec![],
            outputs: vec![],
            attachment_ids: vec![$(
                FunctionAttachmentId {
                    id: String::from($attach),
                }
            ),*],
            host_folder_mounts: vec![],
        }
    }};
}

#[macro_export]
macro_rules! exec_env {
    () => {{
        $crate::exec_env!("exec_env")
    }};
    ($name:expr) => {{
        gbk_protocols::functions::ExecutionEnvironment {
            name: String::from($name),
            entrypoint: String::new(),
            args: vec![],
        }
    }};
}

#[macro_export]
macro_rules! register_attachment_request {
    ($name:expr) => {{
        $crate::register_attachment_request!($name, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", {})
    }};

    ($name:expr, $sha256:expr) => {{
        $crate::register_attachment_request!($name, $sha256, {})
    }};

    ($name:expr, $sha256:expr, {$($key:expr => $value:expr),*}) => {{
        let mut m = ::std::collections::HashMap::new();
        $(
                m.insert(String::from($key), String::from($value));
        )*
        gbk_protocols::functions::RegisterAttachmentRequest {
            name: String::from($name),
            metadata: m,
            checksums: Some(gbk_protocols::functions::Checksums { sha256: String::from($sha256) }),
        }
    }};
}

#[macro_export]
macro_rules! list_request {
    () => {{
        $crate::list_request!("")
    }};

    ($name:expr) => {{
        $crate::list_request!($name, 100, {})
    }};

    ($name:expr, {$($key:expr => $value:expr),*}) => {{
        $crate::list_request!($name, 100, 0, {$($key => $value),*})
    }};

    ($name:expr, {$($key:expr => $value:expr),*}, [$($key_only:expr),*]) => {{
        $crate::list_request!($name, 100, 0, {$($key => $value),*}, [$($key_only),*])
    }};

    ($name:expr, $limit:expr, {$($key:expr => $value:expr),*}) => {{
        $crate::list_request!($name, $limit, 0, {$($key => $value),*})
    }};

    ($name:expr, $limit:expr, $offset:expr, {$($key:expr => $value:expr),*}) => {{
        $crate::list_request!($name, $limit, $offset, {$($key => $value),*}, [])
    }};

    ($name:expr, $limit:expr, $offset:expr, {$($key:expr => $value:expr),*}, [$($only_key:expr),*]) =>
    {{
        let mut metadata = ::std::collections::HashMap::new();
        $(
            metadata.insert(String::from($key), String::from($value));
        )*
        gbk_protocols::functions::ListRequest {
            name_filter: String::from($name),
            metadata_filter: metadata,
            metadata_key_filter: vec![$($only_key),*],
            offset: $offset as u32,
            limit: $limit as u32,
            exact_name_match: false,
            version_requirement: None,
            order_direction: gbk_protocols::functions::OrderingDirection::Descending as i32,
            order_by: gbk_protocols::functions::OrderingKey::Name as i32,
        }
    }};
}
