use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInterface {
    pub name: String,
    pub ip: String,
    pub is_ipv6: bool,
    pub is_loopback: bool,
}

pub fn list_interfaces() -> Vec<NetworkInterface> {
    let mut result = Vec::new();
    match local_ip_address::list_afinet_netifas() {
        Ok(list) => {
            for (name, ip) in list {
                let is_ipv6 = ip.is_ipv6();
                let is_loopback = ip.is_loopback();
                result.push(NetworkInterface {
                    name,
                    ip: ip.to_string(),
                    is_ipv6,
                    is_loopback,
                });
            }
        }
        Err(e) => {
            tracing::warn!("failed to list network interfaces: {e}");
        }
    }
    result
}
