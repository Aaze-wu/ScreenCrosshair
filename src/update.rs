//! 版本与更新检测（**只提示，不下载**）。
//!
//! 设计上有三条硬约束：
//! 1. **只读**：拿 GitHub Releases 的 `tag_name` 比一下版本号，比当前新就显示
//!    一条提示 + 一个「打开发布页」按钮。绝不在程序里下载或安装任何东西——
//!    一个常驻的透明覆盖层没有权限、也没有理由去动自己的可执行文件。
//! 2. **不添依赖**：用系统自带的 WinHTTP 发一个 `GET`，而不是引 `reqwest`
//!    （它会把整个 tokio/hyper/rustls 拖进来，体积比本程序本体还大）。
//! 3. **绝不阻塞界面**：请求在独立线程里跑，结果写回 `Shared`。
//!
//! 版本比较、URL 拼装、返回体解析都被拆成纯函数，可以在单测里跑；
//! 只有 `http_get` 是真的碰系统 API。

// ---------------------------------------------------------------------------
// 常量与纯函数（可跨平台测试）
// ---------------------------------------------------------------------------

/// 编译期配置的仓库地址（`SCREENCROSSHAIR_REPO=owner/repo cargo build --release`）。
///
/// 用 `option_env!` 而不是 `env!`：没设这个变量也要能编译通过，方便直接
/// `cargo build` 试玩。
const BUILD_SLUG: Option<&str> = option_env!("SCREENCROSSHAIR_REPO");

/// 占位符：仓库还没定下来时用的假坐标，仅供文档/测试引用。
///
/// **它永远不会被拿去发请求**：`is_slug` 明确把它判为不可用。
/// 保留它是因为「什么时候该拒绝」本身是需要测的行为。
pub const PLACEHOLDER_SLUG: &str = "OWNER/ScreenCrosshair";

/// 内置的默认仓库坐标。
///
/// 有了它，直接 `cargo build --release` 出来的程序就能做更新检查，
/// 不需要任何环境变量；想改用它 fork 的仓库时，设 `SCREENCROSSHAIR_REPO`
/// 或在配置文件的 `update_repo` 里写一行即可。
pub const DEFAULT_SLUG: &str = "Aaze-wu/ScreenCrosshair";

/// 判定一个 `owner/repo` 是否是**可用**的仓库坐标。
///
/// 只接受 `GitHub` 的实际字符集（字母数字 + `- _ .`），并且必须恰好一段斜杠。
/// 挡得严一点是有意义的：这个字符串会被拼进 URL 交给系统发出去，
/// 顺手接受 `a/b?x=` 这类输入等于给自己开一个注入口子。
pub fn is_slug(s: &str) -> bool {
    let mut parts = s.split('/');
    let owner = parts.next().unwrap_or("");
    let repo = parts.next().unwrap_or("");
    if parts.next().is_some() {
        return false;
    }
    if owner.is_empty() || repo.is_empty() {
        return false;
    }
    // 占位符等于「没配置」，不能拿去请求。
    if owner.eq_ignore_ascii_case("owner") && repo.eq_ignore_ascii_case("ScreenCrosshair") {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
}

/// 解析出实际要用的仓库坐标。
///
/// 优先级：配置文件的 `update_repo` > 编译期 `SCREENCROSSHAIR_REPO` > 内置默认。
/// 前两级都不合法时**退回默认**，而不是「不做检查」——用户装完就该能用，
/// 不该因为配置文件里手抖写了个错值就永久静默。
pub fn repo_slug(configured: Option<&str>) -> Option<String> {
    let candidates = [
        configured.map(str::trim),
        BUILD_SLUG.map(str::trim),
        Some(DEFAULT_SLUG),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|raw| is_slug(raw))
        .map(str::to_string)
}

/// 给人看的发布页（`/releases` 而不是某个具体 tag：用户可能想翻历史版本）。
pub fn releases_url(slug: &str) -> String {
    format!("https://github.com/{slug}/releases")
}

/// 给程序看的 API 地址。
pub fn api_latest_url(slug: &str) -> String {
    format!("https://api.github.com/repos/{slug}/releases/latest")
}

/// 把 `v1.2.3` / `1.2` / `1.2.3-beta.1+sha` 都解析成三段数字。
///
/// **不实现完整 semver**：这里只回答「谁的版本号更新」，而预发布标记
/// 对我们没有意义（我们自己发布的 tag 也未必带 `v` 前缀，带 `-rc` 的更少）。
/// 解析失败返回 `None`，调用方把它当成「不认识的版本」而不是「更新」。
pub fn parse_version(text: &str) -> Option<[u32; 3]> {
    let t = text.trim();
    let t = t.strip_prefix(['v', 'V']).unwrap_or(t);
    // 砍掉 `-beta.1` / `+sha` 这类尾巴
    let core = t.split(['-', '+']).next().unwrap_or(t);
    let mut out = [0u32; 3];
    let mut count = 0usize;
    for piece in core.split('.') {
        if count == 3 {
            break;
        }
        out[count] = piece.trim().parse().ok()?;
        count += 1;
    }
    if count == 0 { None } else { Some(out) }
}

/// 比较两个版本号。解析不出来的当作「更旧」，宁可提示「有新版」也不要静默。
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        (Some(_), None) => true,
        _ => false,
    }
}

/// GitHub `releases/latest` 返回体里我们关心的两个字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub url: String,
}

/// 解析 `releases/latest` 的 JSON。
///
/// 用 `serde_json` 而不是手写字符串查找：返回体里 `body` 字段可能包含任意
/// Markdown（包括 `"tag_name"` 这几个字），字符串搜索会认错。
pub fn parse_release(body: &str) -> Result<Release, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|err| format!("返回内容不是 JSON：{err}"))?;
    // 仓库不存在 / 限流时 GitHub 返回的是 `{"message": "Not Found"}`
    if let Some(msg) = value.get("message").and_then(|v| v.as_str()) {
        return Err(format!("GitHub：{msg}"));
    }
    let tag = value
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "返回里没有 tag_name".to_string())?
        .to_string();
    let url = value
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(Release { tag, url })
}

/// 检查结果，同时供界面显示。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum UpdateState {
    /// 还没查过
    #[default]
    Idle,
    Checking,
    /// 已经是最新
    Latest {
        current: String,
    },
    /// 发现新版本
    Newer {
        current: String,
        latest: String,
        url: String,
    },
    /// 请求/解析失败（离线、限流、仓库不存在……）
    Failed(String),
    /// 没配置仓库坐标，不做联网检查
    NotConfigured,
}

impl UpdateState {
    /// 界面上一句话描述。
    pub fn describe(&self) -> String {
        match self {
            UpdateState::Idle => "尚未检查更新".to_string(),
            UpdateState::Checking => "正在检查更新…".to_string(),
            UpdateState::Latest { current } => format!("已是最新版本（{current}）"),
            UpdateState::Newer {
                current, latest, ..
            } => format!("发现新版本 {latest}（当前 {current}）"),
            UpdateState::Failed(err) => format!("检查更新失败：{err}"),
            UpdateState::NotConfigured => "未配置仓库地址，跳过联网检查".to_string(),
        }
    }

    /// 是否处于「需要用户注意」的状态（界面用不同颜色显示）。
    pub fn is_problem(&self) -> bool {
        matches!(self, UpdateState::Failed(_))
    }

    pub fn is_newer(&self) -> bool {
        matches!(self, UpdateState::Newer { .. })
    }
}

/// 当前版本号（来自 `Cargo.toml`）。
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// 纯逻辑部分：拿到返回体后决定状态。
pub fn evaluate(body: &str, current: &str) -> UpdateState {
    match parse_release(body) {
        Ok(release) => {
            if is_newer(&release.tag, current) {
                let url = if release.url.is_empty() {
                    String::new()
                } else {
                    release.url
                };
                UpdateState::Newer {
                    current: current.to_string(),
                    latest: release.tag,
                    url,
                }
            } else {
                UpdateState::Latest {
                    current: current.to_string(),
                }
            }
        }
        Err(err) => UpdateState::Failed(err),
    }
}

// ---------------------------------------------------------------------------
// 网络（仅 Windows）
// ---------------------------------------------------------------------------

/// 用 WinHTTP 发一个 `GET https://…`，返回响应体文本。
///
/// 为什么是 WinHTTP 而不是 `TcpStream + 自己实现 TLS`：TLS 不是一个能「顺手写写」
/// 的东西。为什么不是 `reqwest`：为了一个 GET 引入整个 async 生态，release
/// 体积会从 13MB 涨到 20MB 以上，纯亏。
#[cfg(windows)]
fn http_get(url: &str) -> Result<String, String> {
    use windows_sys::Win32::Networking::WinHttp::{
        INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
        WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpReadData,
        WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
    };

    /// 句柄守卫：任何一条提前 `return` 的路径都会把已经拿到的句柄关掉。
    ///
    /// windows-sys 里没有导出 `HINTERNET` 这个名字，WinHTTP 句柄就是一个
    /// 裸指针，所以这里直接用它（下面是类型别名，避免数组里到处都是 `*mut`）。
    type RawHandle = *mut core::ffi::c_void;

    struct Handle(RawHandle);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    WinHttpCloseHandle(self.0);
                }
            }
        }
    }

    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| format!("只支持 https：{url}"))?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };

    let agent = crate::platform::wide(concat!("ScreenCrosshair/", env!("CARGO_PKG_VERSION")));
    let host_w = crate::platform::wide(host);
    let path_w = crate::platform::wide(path);
    let verb_w = crate::platform::wide("GET");
    // GitHub 的 API 要求带 `User-Agent`，否则直接 403。
    let headers = crate::platform::wide(
        "Accept: application/vnd.github+json\r\nUser-Agent: ScreenCrosshair\r\n",
    );

    unsafe {
        let session = Handle(WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            std::ptr::null(),
            std::ptr::null(),
            0,
        ));
        if session.0.is_null() {
            return Err("WinHttpOpen 失败".to_string());
        }
        // 单位毫秒：解析/连接/发送/接收。断网时不能让线程一直挂着。
        WinHttpSetTimeouts(session.0, 5000, 5000, 5000, 8000);

        let connect = Handle(WinHttpConnect(
            session.0,
            host_w.as_ptr(),
            INTERNET_DEFAULT_HTTPS_PORT,
            0,
        ));
        if connect.0.is_null() {
            return Err(format!("连接 {host} 失败"));
        }

        let request = Handle(WinHttpOpenRequest(
            connect.0,
            verb_w.as_ptr(),
            path_w.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ));
        if request.0.is_null() {
            return Err("WinHttpOpenRequest 失败".to_string());
        }

        // dwHeadersLength = -1：表头串以 NUL 结尾，长度由 WinHTTP 自己算。
        if WinHttpSendRequest(
            request.0,
            headers.as_ptr(),
            u32::MAX,
            std::ptr::null(),
            0,
            0,
            0,
        ) == 0
        {
            return Err("请求发送失败（可能没有网络）".to_string());
        }
        if WinHttpReceiveResponse(request.0, std::ptr::null_mut()) == 0 {
            return Err("没有收到响应".to_string());
        }

        let mut body: Vec<u8> = Vec::new();
        let mut buf = vec![0u8; 8192];
        loop {
            let mut read: u32 = 0;
            if WinHttpReadData(
                request.0,
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                buf.len() as u32,
                &mut read,
            ) == 0
            {
                return Err("读取响应失败".to_string());
            }
            if read == 0 {
                break;
            }
            body.extend_from_slice(&buf[..read as usize]);
            // 发布信息就几 KB；超过 1MB 说明返回的不是我们想要的东西。
            if body.len() > 1_000_000 {
                break;
            }
        }
        Ok(String::from_utf8_lossy(&body).into_owned())
    }
}

/// 同步检查一次（会被放到后台线程里跑）。
#[cfg(windows)]
pub fn check_once(slug: &str, current: &str) -> UpdateState {
    let url = api_latest_url(slug);
    match http_get(&url) {
        Ok(body) => evaluate(&body, current),
        Err(err) => UpdateState::Failed(err),
    }
}

/// 在后台线程里检查一次，结果写回 `shared.update`。
///
/// 重复调用是安全的：已经在检查时直接返回（界面上有个按钮，用户很容易连点）。
#[cfg(windows)]
pub fn spawn_check(shared: std::sync::Arc<std::sync::Mutex<crate::app::Shared>>) {
    let (slug, current) = match shared.lock() {
        Ok(mut s) => {
            if matches!(s.update, UpdateState::Checking) {
                return;
            }
            let slug = repo_slug(s.config.update_repo.as_deref());
            if slug.is_none() {
                s.update = UpdateState::NotConfigured;
                return;
            }
            s.update = UpdateState::Checking;
            (slug, current_version().to_string())
        }
        Err(_) => return,
    };
    let Some(slug) = slug else { return };

    std::thread::spawn(move || {
        let state = check_once(&slug, &current);
        if let Ok(mut s) = shared.lock() {
            s.update = state;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parsing_accepts_common_shapes() {
        assert_eq!(parse_version("1.2.3"), Some([1, 2, 3]));
        assert_eq!(parse_version("v1.2.3"), Some([1, 2, 3]));
        assert_eq!(parse_version("V0.1.0"), Some([0, 1, 0]));
        assert_eq!(parse_version("2"), Some([2, 0, 0]));
        assert_eq!(parse_version("1.4"), Some([1, 4, 0]));
        assert_eq!(parse_version("v1.2.3-beta.2"), Some([1, 2, 3]));
        assert_eq!(parse_version("1.2.3+build7"), Some([1, 2, 3]));
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("latest"), None);
    }

    #[test]
    fn version_comparison_is_numeric_not_lexicographic() {
        // 字符串比较会把 "0.10.0" 判成比 "0.9.0" 旧，这是经典陷阱。
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(!is_newer("0.9.0", "0.10.0"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        // 解析不了的一方当作旧
        assert!(is_newer("1.0.0", "unknown"));
        assert!(!is_newer("unknown", "1.0.0"));
    }

    #[test]
    fn slug_validation_rejects_junk_and_placeholder() {
        assert!(is_slug("somebody/ScreenCrosshair"));
        assert!(is_slug("a/b"));
        assert!(is_slug("user-name/repo.name"));
        assert!(is_slug(DEFAULT_SLUG), "内置默认坐标本身必须是合法的");
        assert!(!is_slug(PLACEHOLDER_SLUG));
        assert!(!is_slug(""));
        assert!(!is_slug("a"));
        assert!(!is_slug("/b"));
        assert!(!is_slug("a/"));
        assert!(!is_slug("a/b/c"));
        // 想借 URL 拼出别的东西
        assert!(!is_slug("a/b?x=1"));
        assert!(!is_slug("a/b #frag"));
    }

    #[test]
    fn repo_slug_prefers_config_and_falls_back_to_default() {
        // 配置里的值优先，首尾空白要trim掉。
        assert_eq!(
            repo_slug(Some("  somebody/ScreenCrosshair  ")).as_deref(),
            Some("somebody/ScreenCrosshair")
        );
        // 空白 / 占位符 / 非法值都不能生效，要退回下一级。
        let fallback = BUILD_SLUG.map(str::trim).unwrap_or(DEFAULT_SLUG);
        assert_eq!(repo_slug(Some("   ")).as_deref(), Some(fallback));
        assert_eq!(repo_slug(Some(PLACEHOLDER_SLUG)).as_deref(), Some(fallback));
        assert_eq!(repo_slug(Some("a/b?x=1")).as_deref(), Some(fallback));
        // 什么都没配时用内置默认：装完就能查更新。
        assert_eq!(repo_slug(None).as_deref(), Some(fallback));
    }

    #[test]
    fn urls_point_at_the_right_endpoints() {
        assert_eq!(
            releases_url("a/b"),
            "https://github.com/a/b/releases".to_string()
        );
        assert_eq!(
            api_latest_url("a/b"),
            "https://api.github.com/repos/a/b/releases/latest".to_string()
        );
    }

    #[test]
    fn release_body_is_parsed() {
        let body = r#"{
            "tag_name": "v0.2.0",
            "html_url": "https://github.com/a/b/releases/tag/v0.2.0",
            "body": "顺带一提 tag_name 这个词出现在正文里也不该被认成字段"
        }"#;
        let release = parse_release(body).expect("应能解析");
        assert_eq!(release.tag, "v0.2.0");
        assert_eq!(release.url, "https://github.com/a/b/releases/tag/v0.2.0");
    }

    #[test]
    fn api_error_message_becomes_failure() {
        let state = evaluate(r#"{"message":"Not Found"}"#, "0.1.0");
        assert!(matches!(state, UpdateState::Failed(_)));
        let state = evaluate("这不是 JSON", "0.1.0");
        assert!(matches!(state, UpdateState::Failed(_)));
    }

    #[test]
    fn evaluate_decides_newer_and_latest() {
        let newer = evaluate(r#"{"tag_name":"v0.9.0","html_url":"u"}"#, "0.1.0");
        assert!(newer.is_newer());
        assert_eq!(
            newer,
            UpdateState::Newer {
                current: "0.1.0".to_string(),
                latest: "v0.9.0".to_string(),
                url: "u".to_string(),
            }
        );

        let same = evaluate(r#"{"tag_name":"v0.1.0"}"#, "0.1.0");
        assert_eq!(
            same,
            UpdateState::Latest {
                current: "0.1.0".to_string()
            }
        );
    }

    #[test]
    fn state_descriptions_are_human_readable() {
        assert!(UpdateState::NotConfigured.describe().contains("未配置"));
        assert!(UpdateState::Checking.describe().contains("正在检查"));
        let newer = UpdateState::Newer {
            current: "0.1.0".to_string(),
            latest: "0.2.0".to_string(),
            url: "u".to_string(),
        };
        let text = newer.describe();
        assert!(text.contains("0.2.0") && text.contains("0.1.0"));
        assert!(!UpdateState::Idle.is_problem());
        assert!(UpdateState::Failed("x".to_string()).is_problem());
    }
}
