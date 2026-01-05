#[derive(Clone, Debug)]
pub enum DriverAction {
    SetRoute { route_id: u16 },
    SetDirection { direction: crate::model::Direction },
    SetStation { station_id: u16 },
    NextStation,
    PrevStation,
    SyncConfig,
    UploadNow,
    SetBackend { base_url: String },
}

#[derive(Clone, Debug)]
pub struct StatusPanel {
    pub route_id: u16,
    pub route_name: String,
    pub station_id: u16,
    pub station_name: String,
    pub direction: crate::model::Direction,
    pub tap_mode_label: String,
    pub fare_type_label: String,
    pub cache_count: usize,
    pub wifi_connected: bool,
    pub backend_reachable: bool,
    pub backend_base_url: String,
    pub passenger_tone: crate::model::PassengerTone,
    pub passenger_message: String,
    pub standard_fare: Option<f32>,
    pub last_fare: Option<f32>,
    pub last_fare_label: String,
}

#[derive(Clone, Debug)]
pub struct ActionResult {
    pub success: bool,
    pub message: String,
}

pub fn render_index(status: &StatusPanel) -> String {
    let direction = match status.direction {
        crate::model::Direction::Up => "上行",
        crate::model::Direction::Down => "下行",
    };
    let tone_class = status.passenger_tone.css_class();
    let tone_label = status.passenger_tone.label();
    let standard_fare = format_fare(status.standard_fare);
    let actual_fare = format_fare(status.last_fare);
    let backend_display = if status.backend_base_url.is_empty() {
        "默认"
    } else {
        status.backend_base_url.as_str()
    };
    let backend_value = status.backend_base_url.as_str();
    let route_name = if status.route_name.is_empty() {
        "未同步"
    } else {
        status.route_name.as_str()
    };

    let mut html = String::new();
    html.push_str("<!doctype html><html><head><meta charset=\"utf-8\">");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
    html.push_str("<title>TapTransit Gateway</title>");
    html.push_str("<style>");
    html.push_str(":root{--bg:#0f172a;--panel:#0b1220;--text:#f8fafc;--muted:#94a3b8;");
    html.push_str("--accent:#f59e0b;--stroke:rgba(148,163,184,0.25);");
    html.push_str("--student:#10b981;--elder:#fbbf24;--disabled:#3b82f6;--error:#ef4444;--normal:#64748b;}");
    html.push_str("*{box-sizing:border-box}body{margin:0;font-family:\"Source Han Sans SC\",\"Noto Sans SC\",\"PingFang SC\",\"Microsoft YaHei\",sans-serif;");
    html.push_str("color:var(--text);background:radial-gradient(1200px 400px at 50% -200px,#2563eb22,transparent),");
    html.push_str("linear-gradient(180deg,#0b1220,#111827);}h2{margin:0 0 12px 0;font-weight:600}");
    html.push_str(".screen{padding:24px 20px;border-bottom:1px solid var(--stroke);}"); 
    html.push_str(".passenger{min-height:52vh;display:flex;flex-direction:column;gap:16px;}");
    html.push_str(".tone-normal{background:linear-gradient(135deg,#0b1220,#111827);}"); 
    html.push_str(".tone-student{background:linear-gradient(135deg,rgba(16,185,129,0.55),rgba(15,23,42,0.95));}"); 
    html.push_str(".tone-elder{background:linear-gradient(135deg,rgba(251,191,36,0.55),rgba(15,23,42,0.95));}"); 
    html.push_str(".tone-disabled{background:linear-gradient(135deg,rgba(59,130,246,0.55),rgba(15,23,42,0.95));}"); 
    html.push_str(".tone-error{background:linear-gradient(135deg,rgba(239,68,68,0.6),rgba(15,23,42,0.95));}"); 
    html.push_str(".passenger-header{display:flex;align-items:center;justify-content:space-between;gap:12px;}");
    html.push_str(".badge{padding:6px 12px;border-radius:999px;font-size:12px;letter-spacing:1px;background:rgba(255,255,255,0.12);}");
    html.push_str(".tone-student .badge{background:var(--student);color:#022c22;}");
    html.push_str(".tone-elder .badge{background:var(--elder);color:#422006;}");
    html.push_str(".tone-disabled .badge{background:var(--disabled);}");
    html.push_str(".tone-error .badge{background:var(--error);}");
    html.push_str(".route{font-size:28px;font-weight:700;}"); 
    html.push_str(".station{font-size:38px;font-weight:700;}"); 
    html.push_str(".sub{color:var(--muted);font-size:14px;}");
    html.push_str(".fare-grid{display:grid;grid-template-columns:1fr 1fr;gap:12px;}");
    html.push_str(".fare-card{padding:14px;border-radius:16px;border:1px solid var(--stroke);background:rgba(15,23,42,0.6);}"); 
    html.push_str(".fare-title{font-size:12px;color:var(--muted);text-transform:uppercase;letter-spacing:1px;}"); 
    html.push_str(".fare-value{font-size:32px;font-weight:700;margin-top:6px;}"); 
    html.push_str(".message{padding:12px 16px;border-radius:12px;background:rgba(255,255,255,0.08);font-size:16px;}");
    html.push_str(".driver{padding:20px 20px 28px;display:flex;flex-direction:column;gap:16px;background:var(--panel);}"); 
    html.push_str(".driver-grid{display:grid;gap:12px;grid-template-columns:repeat(auto-fit,minmax(160px,1fr));}");
    html.push_str(".driver-card{padding:12px;border-radius:12px;border:1px solid var(--stroke);background:rgba(2,6,23,0.6);}"); 
    html.push_str("button{padding:10px 14px;border-radius:12px;border:1px solid var(--stroke);background:#111827;color:var(--text);font-weight:600;}");
    html.push_str("button.primary{background:var(--accent);color:#0b1220;border-color:transparent;}");
    html.push_str("form{display:flex;flex-wrap:wrap;gap:8px;align-items:center;}");
    html.push_str("input{padding:10px 12px;border-radius:10px;border:1px solid var(--stroke);background:#0f172a;color:var(--text);min-width:160px;}");
    html.push_str(".status-dot{display:inline-block;width:8px;height:8px;border-radius:50%;margin-right:6px;}");
    html.push_str(".dot-ok{background:#22c55e}.dot-bad{background:#f97316}");
    html.push_str("@media (max-width:600px){.fare-grid{grid-template-columns:1fr;}.station{font-size:32px;}.route{font-size:22px;}}");
    html.push_str("</style>");
    html.push_str("</head><body>");
    html.push_str("<section class=\"screen passenger ");
    html.push_str(tone_class);
    html.push_str("\">");
    html.push_str("<div class=\"passenger-header\">");
    html.push_str("<div class=\"route\">线路 ");
    html.push_str(&status.route_id.to_string());
    html.push_str(" · ");
    html.push_str(route_name);
    html.push_str(" · ");
    html.push_str(direction);
    html.push_str("</div>");
    html.push_str("<div class=\"badge\">");
    html.push_str(tone_label);
    html.push_str("</div>");
    html.push_str("</div>");
    html.push_str("<div class=\"station\">");
    html.push_str(&status.station_name);
    html.push_str(" (#");
    html.push_str(&status.station_id.to_string());
    html.push_str(")</div>");
    html.push_str("<div class=\"sub\">下一站由司机切换，屏幕将同步更新</div>");
    html.push_str("<div class=\"fare-grid\">");
    html.push_str("<div class=\"fare-card\">");
    html.push_str("<div class=\"fare-title\">标准票价</div>");
    html.push_str("<div class=\"fare-value\">");
    html.push_str(&standard_fare);
    html.push_str("</div></div>");
    html.push_str("<div class=\"fare-card\">");
    html.push_str("<div class=\"fare-title\">实际票价 · ");
    html.push_str(&status.last_fare_label);
    html.push_str("</div>");
    html.push_str("<div class=\"fare-value\">");
    html.push_str(&actual_fare);
    html.push_str("</div></div>");
    html.push_str("</div>");
    html.push_str("<div class=\"message\">");
    html.push_str(&status.passenger_message);
    html.push_str("</div>");
    html.push_str("</section>");

    html.push_str("<section class=\"driver\">");
    html.push_str("<h2>司机控制面板</h2>");
    html.push_str("<div class=\"driver-grid\">");
    html.push_str("<div class=\"driver-card\"><div class=\"sub\">线路</div><div class=\"route\">");
    html.push_str(&status.route_id.to_string());
    html.push_str("</div><div class=\"sub\">");
    html.push_str(route_name);
    html.push_str("</div></div>");
    html.push_str("<div class=\"driver-card\"><div class=\"sub\">当前站点</div><div class=\"route\">");
    html.push_str(&status.station_name);
    html.push_str("</div><div class=\"sub\">#");
    html.push_str(&status.station_id.to_string());
    html.push_str("</div></div>");
    html.push_str("<div class=\"driver-card\"><div class=\"sub\">方向</div><div class=\"route\">");
    html.push_str(direction);
    html.push_str("</div></div>");
    html.push_str("<div class=\"driver-card\"><div class=\"sub\">刷卡模式</div><div class=\"route\">");
    html.push_str(&status.tap_mode_label);
    html.push_str("</div></div>");
    html.push_str("<div class=\"driver-card\"><div class=\"sub\">票价模式</div><div class=\"route\">");
    html.push_str(&status.fare_type_label);
    html.push_str("</div></div>");
    html.push_str("<div class=\"driver-card\"><div class=\"sub\">缓存条目</div><div class=\"route\">");
    html.push_str(&status.cache_count.to_string());
    html.push_str("</div></div>");
    html.push_str("<div class=\"driver-card\"><div class=\"sub\">Wi-Fi</div><div>");
    html.push_str(if status.wifi_connected { "<span class=\"status-dot dot-ok\"></span>已连接" } else { "<span class=\"status-dot dot-bad\"></span>未连接" });
    html.push_str("</div></div>");
    html.push_str("<div class=\"driver-card\"><div class=\"sub\">后端</div><div>");
    html.push_str(if status.backend_reachable { "<span class=\"status-dot dot-ok\"></span>可达" } else { "<span class=\"status-dot dot-bad\"></span>不可达" });
    html.push_str("</div></div>");
    html.push_str("<div class=\"driver-card\"><div class=\"sub\">后端地址</div><div>");
    html.push_str(backend_display);
    html.push_str("</div></div>");
    html.push_str("</div>");

    html.push_str("<div class=\"driver-grid\">");
    html.push_str("<button onclick=\"location.href='/action?type=prev'\">上一站</button>");
    html.push_str("<button onclick=\"location.href='/action?type=next'\">下一站</button>");
    html.push_str("<button onclick=\"location.href='/action?type=dir_up'\">上行</button>");
    html.push_str("<button onclick=\"location.href='/action?type=dir_down'\">下行</button>");
    html.push_str("<button class=\"primary\" onclick=\"location.href='/action?type=sync'\">同步配置</button>");
    html.push_str("<button onclick=\"location.href='/action?type=upload'\">立即上报</button>");
    html.push_str("</div>");

    html.push_str("<form action=\"/action\" method=\"get\">");
    html.push_str("<input type=\"hidden\" name=\"type\" value=\"set_route\">");
    html.push_str("<input name=\"route_id\" type=\"number\" min=\"0\" placeholder=\"线路ID\">");
    html.push_str("<button type=\"submit\">切换线路</button>");
    html.push_str("</form>");
    html.push_str("<form action=\"/action\" method=\"get\">");
    html.push_str("<input type=\"hidden\" name=\"type\" value=\"set_station\">");
    html.push_str("<input name=\"station_id\" type=\"number\" min=\"0\" placeholder=\"站点ID\">");
    html.push_str("<button type=\"submit\">切换站点</button>");
    html.push_str("</form>");
    html.push_str("<form action=\"/action\" method=\"get\">");
    html.push_str("<input type=\"hidden\" name=\"type\" value=\"set_backend\">");
    html.push_str("<input name=\"backend\" type=\"text\" placeholder=\"后端地址，如 172.20.1.5:80\" value=\"");
    html.push_str(backend_value);
    html.push_str("\">");
    html.push_str("<button type=\"submit\">更新后端</button>");
    html.push_str("</form>");
    html.push_str("</section>");
    html.push_str("</body></html>");
    html
}

pub fn parse_action(query: &str) -> Option<DriverAction> {
    let action_type = query_value(query, "type")?;
    match action_type.as_str() {
        "next" => Some(DriverAction::NextStation),
        "prev" => Some(DriverAction::PrevStation),
        "dir_up" => Some(DriverAction::SetDirection {
            direction: crate::model::Direction::Up,
        }),
        "dir_down" => Some(DriverAction::SetDirection {
            direction: crate::model::Direction::Down,
        }),
        "sync" => Some(DriverAction::SyncConfig),
        "upload" => Some(DriverAction::UploadNow),
        "set_route" => {
            let route_id = query_value(query, "route_id")?.parse().ok()?;
            Some(DriverAction::SetRoute { route_id })
        }
        "set_station" => {
            let station_id = query_value(query, "station_id")?.parse().ok()?;
            Some(DriverAction::SetStation { station_id })
        }
        "set_backend" => {
            let base_url = query_value(query, "backend")?;
            if base_url.is_empty() {
                None
            } else {
                Some(DriverAction::SetBackend { base_url })
            }
        }
        _ => None,
    }
}

fn query_value(query: &str, key: &str) -> Option<String> {
    for part in query.split('&') {
        let mut iter = part.splitn(2, '=');
        let k = iter.next()?;
        let v = iter.next().unwrap_or("");
        if k == key {
            return Some(decode_component(v));
        }
    }
    None
}

fn decode_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_value(bytes[i + 1]);
                let lo = hex_value(bytes[i + 2]);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi << 4 | lo) as char);
                    i += 3;
                } else {
                    out.push(bytes[i] as char);
                    i += 1;
                }
            }
            _ => {
                out.push(bytes[i] as char);
                i += 1;
            }
        }
    }
    out
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn format_fare(fare: Option<f32>) -> String {
    match fare {
        Some(amount) => format!("¥{:.2}", amount),
        None => "—".to_string(),
    }
}
