mod gbk;

fn main() {
    let maya_version = gbk::get_input("version").unwrap_or_else(|_| "2019".to_owned());

    println!("Hello! I will start maya {} from WASI now!", maya_version);

    match gbk::start_host_process(&format!("/usr/autodesk/maya{}/bin/maya", maya_version)) {
        Ok(pid) => {
            println!("started maya");
            gbk::set_output(&gbk::ReturnValue {
                name: "pid".to_owned(),
                r#type: gbk::ArgumentType::Int as i32,
                value: pid.to_le_bytes().to_vec(),
            })
            .map_or_else(|e| println!("Failed to set output: {}", e), |_| ()); // 🕍🥿🕍 🎾
        }
        Err(e) => {
            gbk::set_error(&format!("Failed to start maya 🛕 because of: {}", e))
                .map_or_else(|e| println!("failed to set error: {}", e), |_| ());
        }
    };
}
