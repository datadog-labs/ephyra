//! DogStatsD over UDP: counts summed and distribution values packed per
//! metric and tag set between flushes, events framed as they are sent.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    net::{
        SocketAddr,
        UdpSocket,
    },
};

/// A metric and its tags, formatted as they appear on the line.
type Key = (String, String);

/// Metric values waiting for the next flush.
#[derive(Default)]
pub(crate) struct Buffer {
    counts: BTreeMap<Key, u64>,
    gauges: BTreeMap<Key, f64>,
    distributions: BTreeMap<Key, Vec<f64>>,
    values: usize,
}

impl Buffer {
    pub(crate) fn count(&mut self, name: &str, tags: &[&str], n: u64) {
        *self.counts.entry(key(name, tags)).or_default() += n;
        self.values += 1;
    }

    pub(crate) fn gauge(&mut self, name: &str, tags: &[&str], value: f64) {
        self.gauges.insert(key(name, tags), value);
        self.values += 1;
    }

    pub(crate) fn distribution(&mut self, name: &str, tags: &[&str], value: f64) {
        self.distributions.entry(key(name, tags)).or_default().push(value);
        self.values += 1;
    }

    /// How many values were added since the last [`drain`](Self::drain).
    pub(crate) fn len(&self) -> usize {
        self.values
    }

    /// The buffered values as DogStatsD lines no longer than `max_line`
    /// bytes where the protocol allows (a distribution's values are split
    /// over several lines), emptying the buffer.
    pub(crate) fn drain(&mut self, max_line: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for ((name, tags), n) in std::mem::take(&mut self.counts) {
            lines.push(format!("{name}:{n}|c{tags}"));
        }
        for ((name, tags), value) in std::mem::take(&mut self.gauges) {
            lines.push(format!("{name}:{value}|g{tags}"));
        }
        for ((name, tags), values) in std::mem::take(&mut self.distributions) {
            let fixed = name.len() + "|d".len() + tags.len();
            let mut line = name.clone();
            for value in values {
                let mut piece = String::new();
                let _ = write!(piece, ":{value}");
                if line.len() > name.len()
                    && fixed + (line.len() - name.len()) + piece.len() > max_line
                {
                    lines.push(format!("{line}|d{tags}"));
                    line.truncate(name.len());
                }
                line.push_str(&piece);
            }
            lines.push(format!("{line}|d{tags}"));
        }
        self.values = 0;
        lines
    }
}

fn key(name: &str, tags: &[&str]) -> Key {
    let tags = if tags.is_empty() { String::new() } else { format!("|#{}", tags.join(",")) };
    (name.to_string(), tags)
}

/// How bad an [`event`] is.
#[derive(Clone, Copy)]
pub(crate) enum AlertType {
    Error,
    Warning,
}

/// A DogStatsD event line.
pub(crate) fn event(title: &str, text: &str, alert: AlertType, tags: &[&str]) -> String {
    let escape = |s: &str| s.replace('\n', "\\n");
    let (title, text) = (escape(title), escape(text));
    let alert = match alert {
        AlertType::Error => "error",
        AlertType::Warning => "warning",
    };
    let mut line = format!("_e{{{},{}}}:{title}|{text}|t:{alert}", title.len(), text.len());
    if !tags.is_empty() {
        let _ = write!(line, "|#{}", tags.join(","));
    }
    line
}

/// A UDP socket connected to the agent's DogStatsD port.
pub(crate) struct Agent {
    socket: UdpSocket,
    max_packet: usize,
    /// Datagrams the socket refused, and the last error.
    pub(crate) failed: Option<(u64, std::io::Error)>,
}

impl Agent {
    pub(crate) fn connect(addr: SocketAddr, max_packet: usize) -> std::io::Result<Agent> {
        let local: SocketAddr =
            if addr.is_ipv4() { ([0, 0, 0, 0], 0).into() } else { ([0u16; 8], 0).into() };
        let socket = UdpSocket::bind(local)?;
        socket.connect(addr)?;
        Ok(Agent { socket, max_packet, failed: None })
    }

    /// Send `lines`, as many to a datagram as fit in `max_packet` bytes; a
    /// longer line goes alone.
    pub(crate) fn send(&mut self, lines: impl IntoIterator<Item = String>) {
        let mut packet = String::new();
        for line in lines {
            if !packet.is_empty() && packet.len() + 1 + line.len() > self.max_packet {
                self.send_packet(&packet);
                packet.clear();
            }
            if !packet.is_empty() {
                packet.push('\n');
            }
            packet.push_str(&line);
        }
        if !packet.is_empty() {
            self.send_packet(&packet);
        }
    }

    fn send_packet(&mut self, packet: &str) {
        if let Err(e) = self.socket.send(packet.as_bytes()) {
            let failed = self.failed.take().map_or(0, |(n, _)| n);
            self.failed = Some((failed + 1, e));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_sum_and_distributions_pack_within_the_line_limit() {
        let mut buffer = Buffer::default();
        buffer.count("ephyra.cases", &["status:passed"], 1);
        buffer.count("ephyra.cases", &["status:passed"], 2);
        buffer.gauge("ephyra.coverage.lines", &[], 7.0);
        for v in 0..100 {
            buffer.distribution("ephyra.feature", &["feature:len"], f64::from(v));
        }
        assert_eq!(buffer.len(), 103);
        let lines = buffer.drain(64);
        assert_eq!(buffer.len(), 0);
        assert_eq!(lines[0], "ephyra.cases:3|c|#status:passed");
        assert_eq!(lines[1], "ephyra.coverage.lines:7|g");
        let dists = &lines[2..];
        assert!(dists.len() > 1, "{dists:?}");
        assert!(dists.iter().all(|l| l.len() <= 64 && l.ends_with("|d|#feature:len")), "{dists:?}");
        let values: Vec<f64> = dists
            .iter()
            .flat_map(|l| {
                let body = l.strip_prefix("ephyra.feature:").unwrap().split('|').next().unwrap();
                body.split(':').map(|v| v.parse::<f64>().unwrap()).collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(values, (0..100).map(f64::from).collect::<Vec<_>>());
        assert!(buffer.drain(64).is_empty());
    }

    #[test]
    fn events_count_escaped_bytes() {
        let line = event("ephyra: p failed", "a\nb", AlertType::Error, &["property:p"]);
        assert_eq!(line, "_e{16,4}:ephyra: p failed|a\\nb|t:error|#property:p");
    }
}
