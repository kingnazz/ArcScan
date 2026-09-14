import fs from "node:fs";

const path = "src-tauri/src/scanner.rs";
let source = fs.readFileSync(path, "utf8");

function replaceOnce(label, before, after) {
  const first = source.indexOf(before);
  if (first < 0) throw new Error(`${label}: expected source block was not found`);
  if (source.indexOf(before, first + before.length) >= 0) {
    throw new Error(`${label}: source block matched more than once`);
  }
  source = source.replace(before, after);
}

replaceOnce(
  "macOS ping flags",
  `    #[cfg(target_os = "macos")]
    {
        // macOS: -c 1 count, -t <sec> total timeout (min 1s)
        let secs = ms.div_ceil(1000).max(1);
        cmd.args(["-c", "1", "-t", &secs.to_string(), &ip_s]);
    }`,
  `    #[cfg(target_os = "macos")]
    {
        // macOS normally performs name lookups while pinging an address. On a
        // LAN sweep those reverse-DNS lookups can outlive the actual ICMP reply
        // and make a responsive host look silent to our outer timeout. Keep the
        // probe numeric, apply the per-reply wait in milliseconds, and exit as
        // soon as the single reply arrives.
        let secs = ms.div_ceil(1000).max(1);
        cmd.args([
            "-n",
            "-c",
            "1",
            "-o",
            "-W",
            &ms.to_string(),
            "-t",
            &secs.to_string(),
            &ip_s,
        ]);
    }`,
);

replaceOnce(
  "numeric ARP cache read",
  `async fn read_arp_cache() -> HashMap<Ipv4Addr, String> {
    let mut cmd = quiet_command("arp");
    cmd.arg("-a");
    cmd.stdin(std::process::Stdio::null());`,
  `async fn read_arp_cache() -> HashMap<Ipv4Addr, String> {
    let mut cmd = quiet_command("arp");
    #[cfg(windows)]
    cmd.arg("-a");
    #[cfg(not(windows))]
    cmd.args(["-n", "-a"]);
    // BSD/macOS arp tries to resolve every address symbolically unless -n is
    // supplied. On a populated /24 that can exceed this helper's timeout and
    // discard the entire neighbour table, leaving only ICMP/TCP responders in
    // the scan result. Numeric output is both faster and exactly what we parse.
    cmd.stdin(std::process::Stdio::null());`,
);

fs.writeFileSync(path, source);
console.log("Patched macOS local discovery to avoid reverse-DNS stalls.");
