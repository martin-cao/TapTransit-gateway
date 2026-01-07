/// 司机操作动作（由 Web UI 触发）。
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
    StartRecharge { amount_cents: u32 },
    CancelRecharge,
    StartRegister,
    CancelRegister,
}

/// Web UI 展示的状态面板数据。
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
    pub recharge_active: bool,
    pub recharge_amount_cents: Option<u32>,
    pub register_active: bool,
}

/// 操作结果（预留扩展）。
#[derive(Clone, Debug)]
pub struct ActionResult {
    pub success: bool,
    pub message: String,
}

/// 渲染司机网页（手工拼接 HTML，避免引入模板引擎）。
pub fn render_index(status: &StatusPanel) -> String {
    let direction = match status.direction {
        crate::model::Direction::Up => "上行",
        crate::model::Direction::Down => "下行",
    };
    let tone_class = status.passenger_tone.css_class();
    let tone_label = status.passenger_tone.label();
    let standard_fare = format_fare(status.standard_fare);
    let actual_fare = format_fare(status.last_fare);
    let recharge_amount = format_cents(status.recharge_amount_cents);
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
    html.push_str("<section id=\"passenger-screen\" class=\"screen passenger ");
    html.push_str(tone_class);
    html.push_str("\">");
    html.push_str("<div class=\"passenger-header\">");
    html.push_str("<div class=\"route\" id=\"route-line\">线路 ");
    html.push_str(&status.route_id.to_string());
    html.push_str(" · ");
    html.push_str(route_name);
    html.push_str(" · ");
    html.push_str(direction);
    html.push_str("</div>");
    html.push_str("<div class=\"badge\" id=\"passenger-tone-label\">");
    html.push_str(tone_label);
    html.push_str("</div>");
    html.push_str("</div>");
    html.push_str("<div class=\"station\"><span id=\"station-name\">");
    html.push_str(&status.station_name);
    html.push_str("</span> (#");
    html.push_str("<span id=\"station-id\">");
    html.push_str(&status.station_id.to_string());
    html.push_str("</span>");
    html.push_str(")</div>");
    html.push_str("<div class=\"sub\">下一站由司机切换，屏幕将同步更新</div>");
    html.push_str("<div class=\"fare-grid\">");
    html.push_str("<div class=\"fare-card\">");
    html.push_str("<div class=\"fare-title\">标准票价</div>");
    html.push_str("<div class=\"fare-value\" id=\"fare-standard\">");
    html.push_str(&standard_fare);
    html.push_str("</div></div>");
    html.push_str("<div class=\"fare-card\">");
    html.push_str("<div class=\"fare-title\">实际票价 · <span id=\"fare-label\">");
    html.push_str(&status.last_fare_label);
    html.push_str("</span></div>");
    html.push_str("<div class=\"fare-value\" id=\"fare-actual\">");
    html.push_str(&actual_fare);
    html.push_str("</div></div>");
    html.push_str("</div>");
    html.push_str("<div class=\"message\" id=\"passenger-message\">");
    html.push_str(&status.passenger_message);
    html.push_str("</div>");
    html.push_str("</section>");

    html.push_str("<section class=\"driver\">");
    html.push_str("<h2>司机控制面板</h2>");
    html.push_str("<div class=\"driver-grid\">");
    html.push_str("<div class=\"driver-card\"><div class=\"sub\">线路</div><div class=\"route\" id=\"driver-route-id\">");
    html.push_str(&status.route_id.to_string());
    html.push_str("</div><div class=\"sub\">");
    html.push_str("<span id=\"driver-route-name\">");
    html.push_str(route_name);
    html.push_str("</span>");
    html.push_str("</div></div>");
    html.push_str("<div class=\"driver-card\"><div class=\"sub\">当前站点</div><div class=\"route\" id=\"driver-station-name\">");
    html.push_str(&status.station_name);
    html.push_str("</div><div class=\"sub\">#");
    html.push_str("<span id=\"driver-station-id\">");
    html.push_str(&status.station_id.to_string());
    html.push_str("</span>");
    html.push_str("</div></div>");
    html.push_str("<div class=\"driver-card\"><div class=\"sub\">方向</div><div class=\"route\" id=\"driver-direction\">");
    html.push_str(direction);
    html.push_str("</div></div>");
    html.push_str("<div class=\"driver-card\"><div class=\"sub\">刷卡模式</div><div class=\"route\" id=\"driver-tap-mode\">");
    html.push_str(&status.tap_mode_label);
    html.push_str("</div></div>");
    html.push_str("<div class=\"driver-card\"><div class=\"sub\">票价模式</div><div class=\"route\" id=\"driver-fare-type\">");
    html.push_str(&status.fare_type_label);
    html.push_str("</div></div>");
    html.push_str("<div class=\"driver-card\"><div class=\"sub\">缓存条目</div><div class=\"route\" id=\"driver-cache-count\">");
    html.push_str(&status.cache_count.to_string());
    html.push_str("</div></div>");
    html.push_str("<div class=\"driver-card\"><div class=\"sub\">Wi-Fi</div><div>");
    html.push_str("<span id=\"wifi-dot\" class=\"status-dot ");
    html.push_str(if status.wifi_connected { "dot-ok" } else { "dot-bad" });
    html.push_str("\"></span><span id=\"wifi-text\">");
    html.push_str(if status.wifi_connected { "已连接" } else { "未连接" });
    html.push_str("</span>");
    html.push_str("</div></div>");
    html.push_str("<div class=\"driver-card\"><div class=\"sub\">后端</div><div>");
    html.push_str("<span id=\"backend-dot\" class=\"status-dot ");
    html.push_str(if status.backend_reachable { "dot-ok" } else { "dot-bad" });
    html.push_str("\"></span><span id=\"backend-text\">");
    html.push_str(if status.backend_reachable { "可达" } else { "不可达" });
    html.push_str("</span>");
    html.push_str("</div></div>");
    html.push_str("<div class=\"driver-card\"><div class=\"sub\">后端地址</div><div>");
    html.push_str("<span id=\"backend-address\">");
    html.push_str(backend_display);
    html.push_str("</span>");
    html.push_str("</div></div>");
    html.push_str("<div class=\"driver-card\"><div class=\"sub\">充值模式</div><div class=\"route\" id=\"recharge-status\">");
    html.push_str(if status.recharge_active { "进行中" } else { "未开启" });
    html.push_str("</div><div class=\"sub\">金额 <span id=\"recharge-amount\">");
    html.push_str(&recharge_amount);
    html.push_str("</span></div></div>");
    html.push_str("<div class=\"driver-card\"><div class=\"sub\">注册模式</div><div class=\"route\" id=\"register-status\">");
    html.push_str(if status.register_active { "进行中" } else { "未开启" });
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
    html.push_str("<input id=\"backend-input\" name=\"backend\" type=\"text\" placeholder=\"后端地址，如 172.20.1.5:80\" value=\"");
    html.push_str(backend_value);
    html.push_str("\">");
    html.push_str("<button type=\"submit\">更新后端</button>");
    html.push_str("</form>");
    html.push_str("<form action=\"/action\" method=\"get\">");
    html.push_str("<input type=\"hidden\" name=\"type\" value=\"recharge\">");
    html.push_str("<input name=\"amount\" type=\"number\" step=\"0.01\" min=\"0\" placeholder=\"充值金额(元)\">");
    html.push_str("<button type=\"submit\">进入充值模式</button>");
    html.push_str("</form>");
    html.push_str("<form action=\"/action\" method=\"get\">");
    html.push_str("<input type=\"hidden\" name=\"type\" value=\"recharge_off\">");
    html.push_str("<button type=\"submit\">取消充值模式</button>");
    html.push_str("</form>");
    html.push_str("<form action=\"/action\" method=\"get\">");
    html.push_str("<input type=\"hidden\" name=\"type\" value=\"register_on\">");
    html.push_str("<button type=\"submit\">进入注册模式</button>");
    html.push_str("</form>");
    html.push_str("<form action=\"/action\" method=\"get\">");
    html.push_str("<input type=\"hidden\" name=\"type\" value=\"register_off\">");
    html.push_str("<button type=\"submit\">取消注册模式</button>");
    html.push_str("</form>");
    html.push_str("</section>");
    html.push_str("<script>");
    html.push_str("const toneClasses=['tone-normal','tone-student','tone-elder','tone-disabled','tone-error'];");
    html.push_str("const el=(id)=>document.getElementById(id);");
    html.push_str("function formatFare(v){if(v===null||v===undefined)return '—';return '¥'+Number(v).toFixed(2);}");
    html.push_str("function formatCents(v){if(v===null||v===undefined)return '—';return '¥'+(Number(v)/100).toFixed(2);}");
    html.push_str("function applyStatus(s){");
    html.push_str("const routeName=s.route_name||'未同步';");
    html.push_str("el('route-line').textContent=`线路 ${s.route_id} · ${routeName} · ${s.direction}`;");
    html.push_str("el('station-name').textContent=s.station_name;");
    html.push_str("el('station-id').textContent=s.station_id;");
    html.push_str("el('passenger-tone-label').textContent=s.passenger.tone_label;");
    html.push_str("el('passenger-message').textContent=s.passenger.message;");
    html.push_str("el('fare-standard').textContent=formatFare(s.fare.standard);");
    html.push_str("el('fare-actual').textContent=formatFare(s.fare.actual);");
    html.push_str("el('fare-label').textContent=s.fare.label;");
    html.push_str("el('driver-route-id').textContent=s.route_id;");
    html.push_str("el('driver-route-name').textContent=routeName;");
    html.push_str("el('driver-station-name').textContent=s.station_name;");
    html.push_str("el('driver-station-id').textContent=s.station_id;");
    html.push_str("el('driver-direction').textContent=s.direction;");
    html.push_str("el('driver-tap-mode').textContent=s.tap_mode_label;");
    html.push_str("el('driver-fare-type').textContent=s.fare_type_label;");
    html.push_str("el('driver-cache-count').textContent=s.cache_count;");
    html.push_str("el('wifi-text').textContent=s.wifi_connected?'已连接':'未连接';");
    html.push_str("el('wifi-dot').className='status-dot '+(s.wifi_connected?'dot-ok':'dot-bad');");
    html.push_str("el('backend-text').textContent=s.backend_reachable?'可达':'不可达';");
    html.push_str("el('backend-dot').className='status-dot '+(s.backend_reachable?'dot-ok':'dot-bad');");
    html.push_str("el('backend-address').textContent=s.backend_base_url||'默认';");
    html.push_str("el('recharge-status').textContent=s.recharge_active?'进行中':'未开启';");
    html.push_str("el('recharge-amount').textContent=formatCents(s.recharge_amount_cents);");
    html.push_str("el('register-status').textContent=s.register_active?'进行中':'未开启';");
    html.push_str("const input=document.activeElement;const backendInput=el('backend-input');");
    html.push_str("if(input!==backendInput){backendInput.value=s.backend_base_url||'';}");
    html.push_str("const screen=el('passenger-screen');toneClasses.forEach(c=>screen.classList.remove(c));");
    html.push_str("screen.classList.add(s.passenger.tone_class);");
    html.push_str("}");
    html.push_str("async function refresh(){try{const r=await fetch('/status',{cache:'no-store'});");
    html.push_str("if(!r.ok)return;const s=await r.json();applyStatus(s);}catch(e){}}");
    html.push_str("refresh();setInterval(refresh,1000);");
    html.push_str("</script>");
    html.push_str("</body></html>");
    html
}

/// 解析 URL 查询字符串为 DriverAction。
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
        "recharge" => {
            let amount = query_value(query, "amount")?;
            let amount_cents = parse_amount_cents(&amount)?;
            Some(DriverAction::StartRecharge { amount_cents })
        }
        "recharge_off" => Some(DriverAction::CancelRecharge),
        "register_on" => Some(DriverAction::StartRegister),
        "register_off" => Some(DriverAction::CancelRegister),
        _ => None,
    }
}

/// 获取查询参数值（未进行 URL 解码）。
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

/// URL 解码（处理 %xx 与 +）。
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

/// 十六进制字符转数值。
fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// 票价格式化为人民币显示。
fn format_fare(fare: Option<f32>) -> String {
    match fare {
        Some(amount) => format!("¥{:.2}", amount),
        None => "—".to_string(),
    }
}

/// 金额（分）格式化为人民币。
fn format_cents(amount_cents: Option<u32>) -> String {
    match amount_cents {
        Some(amount) => format!("¥{:.2}", amount as f64 / 100.0),
        None => "—".to_string(),
    }
}

/// 解析充值金额（元）为分。
fn parse_amount_cents(input: &str) -> Option<u32> {
    let value: f64 = input.trim().parse().ok()?;
    if value <= 0.0 {
        return None;
    }
    let cents = (value * 100.0).round();
    if cents <= 0.0 || cents > u32::MAX as f64 {
        return None;
    }
    Some(cents as u32)
}
