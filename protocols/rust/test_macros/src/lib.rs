pub use firm_protocols;

#[macro_export]
macro_rules! attachment {
    () => {{
        $crate::attachment!("fake://")
    }};

    ($url:expr) => {{
        $crate::attachment!(
            $url,
            "FakeAttachment",
            "724a8940e46ffa34e930258f708d890dbb3b3243361dfbc41eefcff124407a29"
        )
    }}; 

    ($url:expr, $name:expr) => {{
        $crate::attachment!(
            $url,
            $name,
            "724a8940e46ffa34e930258f708d890dbb3b3243361dfbc41eefcff124407a29"
        )
    }};

    ($url:expr, $name:expr, $sha256:expr) => {{
        $crate::firm_protocols::functions::Attachment {
            name: $name.to_owned(),
            url: Some($crate::firm_protocols::functions::AttachmentUrl {
                url: $url.to_owned(),
                auth_method: $crate::firm_protocols::functions::AuthMethod::None as i32,
            }),
            metadata: std::collections::HashMap::new(),
            checksums: Some($crate::firm_protocols::functions::Checksums {
                sha256: $sha256.to_owned(),
            }),
            created_at: 0u64,
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
        $crate::attachment!(
            format!("file://{}", path.display()),
            $name,
            hex::encode(sha256)
        )
    }};
}

#[macro_export]
macro_rules! input {
    ($name:expr, $argtype:path) => {{
        $crate::input!($name, false, $argtype)
    }};

    ($name:expr, $required:expr, $argtype:path) => {{
        $crate::firm_protocols::functions::Input {
            name: String::from($name),
            description: String::from($name),
            required: $required,
            r#type: $argtype as i32,
        }
    }};
}

#[macro_export]
macro_rules! output {
    ($name:expr, $argtype:path) => {{
        $crate::firm_protocols::functions::Output {
            name: String::from($name),
            r#type: $argtype as i32,
            description: String::from($name), //"description" (dr evil quotes)
        }
    }};
}

#[macro_export]
macro_rules! function_data {

    ($name:expr, $version:expr) => {{
        $crate::function_data!($name, $version, runtime!())
    }};

    ($name:expr, $version:expr, $runtime:expr) => {{
        $crate::function_data!($name, $version, $runtime, {})
    }};

    ($name:expr, $version:expr, $runtime:expr, {$($key:expr => $value:expr),*}) => {{
        $crate::function_data!($name, $version, $runtime, None, {$($key => $value),*})
    }};

    ($name:expr, $version:expr, $runtime:expr, $code:expr, {$($key:expr => $value:expr),*}) => {{
        $crate::function_data!($name, $version, $runtime, $code, [], {$($key => $value),*})
    }};

    ($name:expr, $version:expr, $runtime:expr, $code:expr, [$($attach:expr),*], {$($key:expr => $value:expr),*}) => {{
        $crate::function_data!($name, $version, $runtime, $code, None, None, [$($attach),*], {$($key => $value),*})
    }};

    ($name:expr, $version:expr, $runtime:expr, $code:expr, $input:expr, $output:expr, [$($attach:expr),*], {$($key:expr => $value:expr),*}) => {{
        let mut metadata = ::std::collections::HashMap::new();
        $(
            metadata.insert(String::from($key), String::from($value));
        )*
        $crate::firm_protocols::registry::FunctionData {
            name: String::from($name),
            version: String::from($version),
            runtime: ::std::option::Option::from($runtime),
            code_attachment_id: ::std::option::Option::from($code),
            metadata,
            input: ::std::option::Option::from($input),
            output: ::std::option::Option::from($output),
            attachment_ids: vec![$(($attach),)*].into_iter().collect(),
        }
    }};
}

#[macro_export]
macro_rules! runtime {
    () => {{
        $crate::runtime!("runtime")
    }};
    ($name:expr) => {{
        $crate::firm_protocols::functions::Runtime {
            name: String::from($name),
            entrypoint: String::new(),
            arguments: ::std::collections::HashMap::new(),
        }
    }};
}

#[macro_export]
macro_rules! attachment_data {
    ($name:expr) => {{
        $crate::attachment_data!($name, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", {})
    }};

    ($name:expr, $sha256:expr) => {{
        $crate::attachment_data!($name, $sha256, {})
    }};

    ($name:expr, $sha256:expr, {$($key:expr => $value:expr),*}) => {{
        let mut m = ::std::collections::HashMap::new();
        $(
                m.insert(String::from($key), String::from($value));
        )*
        $crate::firm_protocols::registry::AttachmentData {
            name: String::from($name),
            metadata: m,
            checksums: Some($crate::firm_protocols::functions::Checksums { sha256: String::from($sha256) }),
        }
    }};
}

#[macro_export]
macro_rules! filters {
    () => {{
        $crate::filters!("")
    }};

    ($name:expr) => {{
        $crate::filters!($name, 100, {})
    }};

    ($name:expr, {$($key:expr => $value:expr),*}) => {{
        $crate::filters!($name, 100, 0, {$($key => $value),*})
    }};

    ($name:expr, {$($key:expr => $value:expr),*}, [$($key_only:expr),*]) => {{
        $crate::filters!($name, 100, 0, {$($key => $value),*}, [$($key_only),*])
    }};

    ($name:expr, $limit:expr, {$($key:expr => $value:expr),*}) => {{
        $crate::filters!($name, $limit, 0, {$($key => $value),*})
    }};

    ($name:expr, $limit:expr, $offset:expr, {$($key:expr => $value:expr),*}) => {{
        $crate::filters!($name, $limit, $offset, {$($key => $value),*}, [])
    }};

    ($name:expr, $limit:expr, $offset:expr, {$($key:expr => $value:expr),*}, [$($only_key:expr),*]) =>
    {{
        let mut metadata = ::std::collections::HashMap::new();
        $(
            metadata.insert(String::from($key), String::from($value));
         )*

        $(
            metadata.insert(String::from($only_key), String::new());
         )*
        $crate::firm_protocols::registry::Filters {
            name: Some($crate::firm_protocols::registry::NameFilter {
                pattern: String::from($name),
                exact_match: false,
            }),
            metadata: metadata,
            order: Some($crate::firm_protocols::registry::Ordering {
                reverse: false,
                key: $crate::firm_protocols::registry::OrderingKey::NameVersion as i32,
                offset: $offset as u32,
                limit: $limit as u32,
            }),
            version_requirement: None,
        }
    }};
}

#[macro_export]
macro_rules! stream {
    () => {{
        $crate::stream!({})
    }};

    ({$($key:expr => $value:expr),*}) => {{
        $crate::firm_protocols::execution::Stream {
            channels: vec![$((String::from($key),$value.to_channel())),*].into_iter().collect()
        }
    }};
}

#[macro_export]
macro_rules! stream_spec {
    ({$($key:expr => $value:expr),*}) => {{
        $crate::stream_spec!({$($key => $value),*}, {})
    }};

    ({$($key:expr => $value:expr),*}, {$($opt_key:expr => $opt_value:expr),*}) => {{
        $crate::firm_protocols::functions::StreamSpec {
            required: vec![$((String::from($key),$value)),*].into_iter().collect(),
            optional: vec![$((String::from($opt_key),$opt_value)),*].into_iter().collect(),
        }
    }};
}
