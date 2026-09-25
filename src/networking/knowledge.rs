//! Static networking reference data: well-known ports and special IPv4
//! ranges. Conceptual knowledge (TCP, DNS, TLS...) lives in the JSON
//! knowledge base; this module holds tables that code computes against.

use std::net::Ipv4Addr;

/// A well-known service port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortInfo {
    pub port: u16,
    pub service: &'static str,
    pub transport: &'static str,
    pub description: &'static str,
}

const fn p(
    port: u16,
    service: &'static str,
    transport: &'static str,
    description: &'static str,
) -> PortInfo {
    PortInfo {
        port,
        service,
        transport,
        description,
    }
}

/// Sorted by port number.
pub const PORTS: &[PortInfo] = &[
    p(20, "FTP (dados)", "tcp", "Canal de dados do FTP ativo"),
    p(
        21,
        "FTP",
        "tcp",
        "Controle do FTP; tráfego sem criptografia",
    ),
    p(22, "SSH", "tcp", "Shell remoto seguro, scp e sftp"),
    p(23, "Telnet", "tcp", "Shell remoto sem criptografia; evite"),
    p(25, "SMTP", "tcp", "Envio de e-mail entre servidores"),
    p(
        53,
        "DNS",
        "udp/tcp",
        "Resolução de nomes; TCP para respostas grandes e transferências de zona",
    ),
    p(67, "DHCP (servidor)", "udp", "Servidor DHCP entrega IPs"),
    p(
        68,
        "DHCP (cliente)",
        "udp",
        "Cliente DHCP recebe configuração",
    ),
    p(
        69,
        "TFTP",
        "udp",
        "Transferência de arquivos trivial (boot PXE)",
    ),
    p(80, "HTTP", "tcp", "Web sem criptografia"),
    p(110, "POP3", "tcp", "Leitura de e-mail (legado)"),
    p(123, "NTP", "udp", "Sincronização de relógio"),
    p(143, "IMAP", "tcp", "Leitura de e-mail"),
    p(161, "SNMP", "udp", "Monitoramento de equipamentos de rede"),
    p(389, "LDAP", "tcp", "Diretório (usuários, grupos)"),
    p(443, "HTTPS", "tcp", "HTTP sobre TLS"),
    p(
        445,
        "SMB",
        "tcp",
        "Compartilhamento de arquivos Windows/Samba",
    ),
    p(465, "SMTPS", "tcp", "SMTP sobre TLS implícito"),
    p(514, "Syslog", "udp", "Envio de logs"),
    p(
        587,
        "SMTP (submission)",
        "tcp",
        "Envio de e-mail por clientes, com STARTTLS",
    ),
    p(636, "LDAPS", "tcp", "LDAP sobre TLS"),
    p(993, "IMAPS", "tcp", "IMAP sobre TLS"),
    p(995, "POP3S", "tcp", "POP3 sobre TLS"),
    p(1433, "SQL Server", "tcp", "Microsoft SQL Server"),
    p(1521, "Oracle", "tcp", "Oracle Database listener"),
    p(2049, "NFS", "tcp/udp", "Sistema de arquivos em rede"),
    p(
        2375,
        "Docker API",
        "tcp",
        "API do Docker sem TLS; nunca exponha",
    ),
    p(2376, "Docker API (TLS)", "tcp", "API do Docker com TLS"),
    p(
        3000,
        "Dev server",
        "tcp",
        "Comum em Node.js, Rails e Grafana",
    ),
    p(3306, "MySQL/MariaDB", "tcp", "Banco de dados MySQL"),
    p(3389, "RDP", "tcp", "Área de trabalho remota do Windows"),
    p(
        5000,
        "Dev server",
        "tcp",
        "Comum em Flask e Docker Registry",
    ),
    p(5173, "Vite", "tcp", "Servidor de desenvolvimento do Vite"),
    p(5432, "PostgreSQL", "tcp", "Banco de dados PostgreSQL"),
    p(5672, "AMQP", "tcp", "RabbitMQ e outros brokers AMQP"),
    p(5900, "VNC", "tcp", "Área de trabalho remota VNC"),
    p(6379, "Redis", "tcp", "Banco em memória Redis"),
    p(6443, "Kubernetes API", "tcp", "API server do Kubernetes"),
    p(
        8000,
        "HTTP (dev)",
        "tcp",
        "Servidores de desenvolvimento (Django, python -m http.server)",
    ),
    p(
        8080,
        "HTTP alternativo",
        "tcp",
        "Proxies, Tomcat, Spring Boot e servidores de desenvolvimento",
    ),
    p(
        8443,
        "HTTPS alternativo",
        "tcp",
        "HTTPS em porta não privilegiada",
    ),
    p(9000, "Diversos", "tcp", "PHP-FPM, SonarQube, MinIO"),
    p(9090, "Prometheus", "tcp", "Servidor Prometheus"),
    p(9092, "Kafka", "tcp", "Broker Apache Kafka"),
    p(9200, "Elasticsearch", "tcp", "API HTTP do Elasticsearch"),
    p(11211, "Memcached", "tcp/udp", "Cache em memória"),
    p(15672, "RabbitMQ (gestão)", "tcp", "Painel web do RabbitMQ"),
    p(27017, "MongoDB", "tcp", "Banco de dados MongoDB"),
];

pub fn port_info(port: u16) -> Option<&'static PortInfo> {
    PORTS
        .binary_search_by_key(&port, |p| p.port)
        .ok()
        .map(|i| &PORTS[i])
}

/// IANA range a port belongs to.
pub fn port_range(port: u16) -> &'static str {
    match port {
        0 => "reservada",
        1..=1023 => "bem conhecida (0–1023): exige root para escutar",
        1024..=49151 => "registrada (1024–49151)",
        _ => "dinâmica/efêmera (49152–65535): usada como porta de origem",
    }
}

/// A special-purpose IPv4 block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressRange {
    pub network: [u8; 4],
    pub prefix: u8,
    pub name: &'static str,
    pub description: &'static str,
}

const fn r(
    network: [u8; 4],
    prefix: u8,
    name: &'static str,
    description: &'static str,
) -> AddressRange {
    AddressRange {
        network,
        prefix,
        name,
        description,
    }
}

/// Most specific first.
pub const IPV4_RANGES: &[AddressRange] = &[
    r(
        [255, 255, 255, 255],
        32,
        "broadcast",
        "Broadcast limitado: todos os hosts do enlace local",
    ),
    r(
        [192, 0, 2, 0],
        24,
        "documentação",
        "TEST-NET-1 (RFC 5737): exemplos em documentação",
    ),
    r(
        [198, 51, 100, 0],
        24,
        "documentação",
        "TEST-NET-2 (RFC 5737): exemplos em documentação",
    ),
    r(
        [203, 0, 113, 0],
        24,
        "documentação",
        "TEST-NET-3 (RFC 5737): exemplos em documentação",
    ),
    r(
        [169, 254, 0, 0],
        16,
        "link-local",
        "Autoconfiguração (sem DHCP); não é roteado",
    ),
    r(
        [192, 168, 0, 0],
        16,
        "privado",
        "Rede privada RFC 1918 (redes domésticas)",
    ),
    r(
        [172, 16, 0, 0],
        12,
        "privado",
        "Rede privada RFC 1918 (inclui a rede padrão do Docker 172.17.0.0/16)",
    ),
    r(
        [100, 64, 0, 0],
        10,
        "CGNAT",
        "Espaço compartilhado de operadoras (RFC 6598)",
    ),
    r(
        [10, 0, 0, 0],
        8,
        "privado",
        "Rede privada RFC 1918 (redes corporativas, nuvem)",
    ),
    r(
        [127, 0, 0, 0],
        8,
        "loopback",
        "Loopback: o próprio host (localhost)",
    ),
    r(
        [0, 0, 0, 0],
        8,
        "não especificado",
        "\"Esta rede\"; 0.0.0.0 em um servidor = todas as interfaces",
    ),
    r([224, 0, 0, 0], 4, "multicast", "Multicast (um para muitos)"),
    r([240, 0, 0, 0], 4, "reservado", "Reservado para uso futuro"),
];

const PUBLIC: AddressRange = r(
    [0, 0, 0, 0],
    0,
    "público",
    "Endereço público, roteável na internet",
);

/// Classifies an IPv4 address by its special-purpose block.
pub fn classify_ipv4(ip: Ipv4Addr) -> &'static AddressRange {
    let addr = u32::from(ip);
    IPV4_RANGES
        .iter()
        .find(|range| {
            let net = u32::from(Ipv4Addr::from(range.network));
            let mask = if range.prefix == 0 {
                0
            } else {
                u32::MAX << (32 - range.prefix)
            };
            addr & mask == net
        })
        .unwrap_or(&PUBLIC)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_are_sorted_and_found() {
        assert!(PORTS.windows(2).all(|w| w[0].port < w[1].port));
        assert_eq!(port_info(8080).unwrap().service, "HTTP alternativo");
        assert_eq!(port_info(22).unwrap().service, "SSH");
        assert!(port_info(12345).is_none());
        assert!(port_range(443).starts_with("bem conhecida"));
        assert!(port_range(51000).starts_with("dinâmica"));
    }

    #[test]
    fn ipv4_classification() {
        let c = |s: &str| classify_ipv4(s.parse().unwrap()).name;
        assert_eq!(c("192.168.1.10"), "privado");
        assert_eq!(c("172.20.0.1"), "privado");
        assert_eq!(c("172.32.0.1"), "público");
        assert_eq!(c("10.1.2.3"), "privado");
        assert_eq!(c("127.0.0.1"), "loopback");
        assert_eq!(c("169.254.1.1"), "link-local");
        assert_eq!(c("8.8.8.8"), "público");
        assert_eq!(c("224.0.0.1"), "multicast");
        assert_eq!(c("100.64.0.1"), "CGNAT");
    }
}
