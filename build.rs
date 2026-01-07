use std::fs;

fn main() {
    // 初始化 ESP-IDF 构建环境变量
    embuild::espidf::sysenv::output();
    // 从 .env 读取编译期配置
    load_dotenv();
}

/// 读取 .env 并注入为编译期环境变量。
fn load_dotenv() {
    const ENV_PATH: &str = ".env";
    println!("cargo:rerun-if-changed={}", ENV_PATH);

    let Ok(contents) = fs::read_to_string(ENV_PATH) else {
        println!("cargo:warning=missing .env (expected WIFI_SSID/WIFI_PASS/BACKEND_BASE_URL)");
        return;
    };

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let mut value = value.trim().to_string();
        if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
            value = value[1..value.len() - 1].to_string();
        }
        // 仅允许白名单字段进入编译期环境
        if matches!(
            key,
            "WIFI_SSID" | "WIFI_PASS" | "BACKEND_BASE_URL" | "DEFAULT_ROUTE_ID"
        ) {
            println!("cargo:rustc-env={}={}", key, value);
        }
    }
}
