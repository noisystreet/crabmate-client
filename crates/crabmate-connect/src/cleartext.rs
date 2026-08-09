//! 明文 HTTP 连接策略：仅本机 / RFC1918 / 链路本地；公网须 HTTPS。

use std::net::IpAddr;

use url::Url;

/// 校验连接目标：`https` 放行；`http` 仅当 host 为 loopback / 私网 / 链路本地。
pub fn enforce_cleartext_connect_policy(url: &Url) -> Result<(), String> {
    match url.scheme() {
        "https" => Ok(()),
        "http" => {
            if host_allows_cleartext(url) {
                Ok(())
            } else {
                Err(
                    "明文 HTTP 仅允许本机或局域网地址（RFC1918 / 链路本地）；公网请使用 HTTPS"
                        .into(),
                )
            }
        }
        other => Err(format!("仅支持 http/https，收到 {other}")),
    }
}

fn host_allows_cleartext(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match url.host() {
        Some(url::Host::Ipv4(v4)) => {
            let ip = IpAddr::V4(v4);
            ip.is_loopback() || ipv4_is_private_or_link_local(v4)
        }
        Some(url::Host::Ipv6(v6)) => {
            let ip = IpAddr::V6(v6);
            ip.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local()
        }
        Some(url::Host::Domain(name)) => {
            // 公网域名明文风险高；仅放行本机名与常见局域网后缀（填 IP 始终可用）。
            let n = name.to_ascii_lowercase();
            n == "localhost"
                || n.ends_with(".localhost")
                || n.ends_with(".local")
                || n.ends_with(".lan")
                || n.ends_with(".home.arpa")
        }
        None => false,
    }
}

fn ipv4_is_private_or_link_local(v4: std::net::Ipv4Addr) -> bool {
    v4.is_private() || v4.is_link_local()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Url {
        Url::parse(s).expect("url")
    }

    #[test]
    fn https_always_ok() {
        assert!(enforce_cleartext_connect_policy(&parse("https://example.com/")).is_ok());
        assert!(enforce_cleartext_connect_policy(&parse("https://1.2.3.4/")).is_ok());
    }

    #[test]
    fn http_private_and_loopback_ok() {
        assert!(enforce_cleartext_connect_policy(&parse("http://127.0.0.1:8080/")).is_ok());
        assert!(enforce_cleartext_connect_policy(&parse("http://localhost:8080/")).is_ok());
        assert!(enforce_cleartext_connect_policy(&parse("http://192.168.1.10:8080/")).is_ok());
        assert!(enforce_cleartext_connect_policy(&parse("http://10.0.0.2:8080/")).is_ok());
        assert!(enforce_cleartext_connect_policy(&parse("http://172.16.5.1:8080/")).is_ok());
    }

    #[test]
    fn http_lan_mdns_ok() {
        assert!(enforce_cleartext_connect_policy(&parse("http://nas.local:8080/")).is_ok());
        assert!(enforce_cleartext_connect_policy(&parse("http://box.home.arpa/")).is_ok());
    }

    #[test]
    fn http_public_rejected() {
        assert!(enforce_cleartext_connect_policy(&parse("http://1.1.1.1/")).is_err());
        assert!(enforce_cleartext_connect_policy(&parse("http://example.com/")).is_err());
    }
}
