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
        $crate::functions::Attachment {
            name: $name.to_owned(),
            url: Some($crate::functions::AttachmentUrl {
                url: $url.to_owned(),
                auth_method: $crate::functions::AuthMethod::None as i32,
            }),
            metadata: std::collections::HashMap::new(),
            checksums: Some($crate::functions::Checksums {
                sha256: $sha256.to_owned(),
            }),
            created_at: 0u64,
            publisher: Some($crate::functions::Publisher {
                name: String::new(),
                email: String::new(),
            }),
            signature: None,
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
        $crate::functions::Input {
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
        $crate::functions::Output {
            name: String::from($name),
            r#type: $argtype as i32,
            description: String::from($name), //"description" (dr evil quotes)
        }
    }};
}

#[macro_export]
macro_rules! function_data {

    ($name:expr, $version:expr) => {{
        $crate::function_data!($name, $version, runtime_spec!())
    }};

    ($name:expr, $version:expr, $runtime_spec:expr) => {{
        $crate::function_data!($name, $version, $runtime_spec, {})
    }};

    ($name:expr, $version:expr, $runtime_spec:expr, {$($key:expr => $value:expr),*}) => {{
        $crate::function_data!($name, $version, $runtime_spec, None, {$($key => $value),*})
    }};

    ($name:expr, $version:expr, $runtime_spec:expr, $code:expr, {$($key:expr => $value:expr),*}) => {{
        $crate::function_data!($name, $version, $runtime_spec, $code, [], {$($key => $value),*})
    }};

    ($name:expr, $version:expr, $runtime_spec:expr, $code:expr, [$($attach:expr),*], {$($key:expr => $value:expr),*}) => {{
        $crate::function_data!(
            $name,
            $version,
            $runtime_spec,
            $code,
            ::std::collections::HashMap::new(),
            ::std::collections::HashMap::new(),
            ::std::collections::HashMap::new(),
            "Someone",
            "someone@example.com",
            [$($attach),*],
            {$($key => $value),*})
    }};

    ($name:expr, $version:expr, $runtime_spec:expr, $code:expr, [$($attach:expr),*], {$($key:expr => $value:expr),*}, $email:expr) => {{
        $crate::function_data!(
            $name,
            $version,
            $runtime_spec,
            $code,
            ::std::collections::HashMap::new(),
            ::std::collections::HashMap::new(),
            ::std::collections::HashMap::new(),
            "Someone",
            $email,
            [$($attach),*],
            {$($key => $value),*})
    }};

    ($name:expr,
     $version:expr,
     $runtime_spec:expr,
     $code:expr,
     $req_inputs:expr,
     $opt_inputs:expr,
     $outputs:expr,
     $publisher_name:expr,
     $publisher_email:expr,
     [$($attach:expr),*],
     {$($key:expr => $value:expr),*}
    ) => {{
        let mut metadata = ::std::collections::HashMap::new();
        $(
            metadata.insert(String::from($key), String::from($value));
        )*
        $crate::functions::FunctionData {
            name: String::from($name),
            version: String::from($version),
            runtime: ::std::option::Option::from($runtime_spec),
            code_attachment_id: ::std::option::Option::from($code),
            metadata,
            required_inputs: $req_inputs,
            optional_inputs: $opt_inputs,
            outputs: $outputs,
            attachment_ids: vec![$(($attach),)*].into_iter().collect(),

            publisher: Some($crate::functions::Publisher {
                name: String::from($publisher_name),
                email: String::from($publisher_email),
            }),
            signature: None,
        }
    }};
}

#[macro_export]
macro_rules! runtime_spec {
    () => {{
        $crate::runtime_spec!("runtime")
    }};
    ($name:expr) => {{
        $crate::functions::RuntimeSpec {
            name: String::from($name),
            entrypoint: String::new(),
            arguments: ::std::collections::HashMap::new(),
        }
    }};
}

#[macro_export]
macro_rules! attachment_data {
    ($name:expr) => {{
        $crate::attachment_data!($name, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
    }};

    ($name:expr, $sha256:expr) => {{
        $crate::attachment_data!($name, $sha256, {})
    }};

    ($name:expr, $sha256:expr, {$($key:expr => $value:expr),*}) => {{
        $crate::attachment_data!($name, $sha256, "Horn Simpa", "hornsimpa@fulfisk.se", {$($key => $value),*})
    }};

    ($name:expr, $sha256:expr, $publisher_name:expr, $publisher_email:expr, {$($key:expr => $value:expr),*}) => {{
        let mut m = ::std::collections::HashMap::new();
        $(
                m.insert(String::from($key), String::from($value));
        )*
        $crate::functions::AttachmentData {
            name: String::from($name),
            metadata: m,
            checksums: Some($crate::functions::Checksums { sha256: String::from($sha256) }),
            publisher: Some($crate::functions::Publisher {
                name: String::from($publisher_name),
                email: String::from($publisher_email),
            }),
            signature: None,
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

    ($name:expr, $limit:expr, $offset:expr, {$($key:expr => $value:expr),*}, [$($only_key:expr),*]) => {{
        $crate::filters!($name, $limit, $offset, {$($key => $value),*}, [$($only_key),*], "")
    }};

    ($name:expr, $limit:expr, $offset:expr, {$($key:expr => $value:expr),*}, [$($only_key:expr),*], $publisher_email:expr) =>
    {{
        let mut metadata = ::std::collections::HashMap::new();
        $(
            metadata.insert(String::from($key), String::from($value));
         )*

        $(
            metadata.insert(String::from($only_key), String::new());
         )*
        $crate::functions::Filters {
            name: String::from($name),
            metadata: metadata,
            order: Some($crate::functions::Ordering {
                reverse: false,
                key: $crate::functions::OrderingKey::NameVersion as i32,
                offset: $offset as u64,
                limit: $limit as u64,
            }),
            version_requirement: None,
            publisher_email: String::from($publisher_email),
        }
    }};
}

#[macro_export]
macro_rules! stream {
    () => {{
        $crate::stream!({})
    }};

    ({$($key:expr => $value:expr),*}) => {{
        $crate::functions::Stream {
            channels: vec![$((String::from($key),$value.to_channel())),*].into_iter().collect()
        }
    }};
}

#[macro_export]
macro_rules! channel_specs {
    ({$($key:expr => $value:expr),*}) => {{
        (vec![$((String::from($key),$value)),*].into_iter().collect(), None::<::std::collections::HashMap<String, $crate::functions::ChannelSpec>>)
    }};

    ({$($key:expr => $value:expr),*}, {$($opt_key:expr => $opt_value:expr),*}) => {{
        (
            vec![$((String::from($key),$value)),*].into_iter().collect(),
            Some(vec![$((String::from($opt_key),$opt_value)),*].into_iter().collect()),
        )
    }};
}
