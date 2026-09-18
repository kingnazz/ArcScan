//! The PowerShell the credentialed probe runs, and how the password reaches it.
//!
//! # Why the password is not an argument
//!
//! On Windows, a process's command line is readable by any other process on the
//! machine running as the same user, and it is what most endpoint-protection
//! products log. `-Credential (… "P@ssw0rd" …)` would put a domain
//! administrator's password in that log on every scanned host's behalf. So the
//! script takes the password on **stdin**, one line, and nothing secret is ever
//! passed as an argument or written to a file.
//!
//! The script body itself carries no secret — only the target and the user name
//! — so it travels as a `-EncodedCommand`, which sidesteps every layer of
//! quoting between Rust, `cmd.exe` and PowerShell without needing a temporary
//! file to clean up.
//!
//! The generator lives here, away from the process launcher, so the shape of
//! the command can be asserted on any platform: the tests below are what keep
//! a future edit from moving the password onto the command line.
//!
//! # Why certificate validation is not disabled
//!
//! On the HTTPS transport the session is built with `-UseSsl` and *without*
//! `-SkipCACheck` or `-SkipCNCheck`. Most WinRM HTTPS listeners carry a
//! self-signed certificate, so this means some of them will fail — and the
//! failure is reported as a certificate problem the operator can act on.
//!
//! The alternative is to turn validation off, which on this particular channel
//! means handing a domain credential to whoever answers the connection. A
//! scanner that silently accepts any certificate in order to look like it
//! works is a credential-harvesting opportunity wearing an inventory tool's
//! name. The 5985 transport, which is preferred whenever it exists, is already
//! message-encrypted by Negotiate and does not have this problem at all.

use super::transport::Transport;

/// Build the collection script for one target and account.
///
/// The returned text reads the password from stdin as its first action and
/// holds it only as a `SecureString`. Every value interpolated into the script
/// is validated by [`super::validate_target`] and
/// [`crate::discovery::windows::creds::WindowsCredential`] before it gets here.
pub fn collection_script(target: &str, account: &str, transport: Transport) -> String {
    // Single-quoted PowerShell strings interpret nothing but a doubled quote,
    // so escaping that one character is the whole escaping rule. The inputs are
    // already restricted to characters that cannot include it; this is the
    // second of the two checks.
    let target = ps_single_quote(target);
    let account = ps_single_quote(account);
    // Built from the selected transport rather than hardcoded. `-UseSsl`
    // carries full certificate validation; see the module note on why.
    let session_option = if transport.uses_ssl() {
        "New-CimSessionOption -Protocol Wsman -UseSsl"
    } else {
        "New-CimSessionOption -Protocol Wsman"
    };
    let port = transport.port();
    format!(
        r#"$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$target = {target}
$account = {account}
try {{
    $plain = [Console]::In.ReadLine()
    if ([string]::IsNullOrEmpty($plain)) {{ throw 'No credential was supplied.' }}
    $secure = ConvertTo-SecureString -String $plain -AsPlainText -Force
    $plain = $null
    [System.GC]::Collect()
    $cred = New-Object System.Management.Automation.PSCredential($account, $secure)

    $opt = {session_option}
    $session = New-CimSession -ComputerName $target -Credential $cred -SessionOption $opt -Port {port} -OperationTimeoutSec 20
    try {{
        $os   = Get-CimInstance -CimSession $session -ClassName Win32_OperatingSystem
        $cs   = Get-CimInstance -CimSession $session -ClassName Win32_ComputerSystem
        $prod = Get-CimInstance -CimSession $session -ClassName Win32_ComputerSystemProduct
        $bios = Get-CimInstance -CimSession $session -ClassName Win32_BIOS
        $nics = Get-CimInstance -CimSession $session -ClassName Win32_NetworkAdapterConfiguration -Filter 'IPEnabled = True'

        $report = [ordered]@{{
            os = [ordered]@{{
                Caption        = $os.Caption
                Version        = $os.Version
                BuildNumber    = $os.BuildNumber
                OSArchitecture = $os.OSArchitecture
                ProductType    = $os.ProductType
            }}
            computer_system = [ordered]@{{
                Name         = $cs.Name
                Manufacturer = $cs.Manufacturer
                Model        = $cs.Model
                Domain       = $cs.Domain
                PartOfDomain = $cs.PartOfDomain
                Workgroup    = $cs.Workgroup
            }}
            product = [ordered]@{{
                UUID             = $prod.UUID
                IdentifyingNumber = $prod.IdentifyingNumber
                Vendor           = $prod.Vendor
                Name             = $prod.Name
            }}
            bios = [ordered]@{{
                SerialNumber = $bios.SerialNumber
                Manufacturer = $bios.Manufacturer
            }}
            nics = @($nics | ForEach-Object {{
                [ordered]@{{
                    MACAddress  = $_.MACAddress
                    IPAddress   = @($_.IPAddress)
                    Description = $_.Description
                }}
            }})
        }}
        $report | ConvertTo-Json -Depth 5 -Compress
    }} finally {{
        Remove-CimSession -CimSession $session -ErrorAction SilentlyContinue
    }}
}} catch {{
    # The message only: a full PowerShell error record carries the invocation
    # line, and the invocation is the one thing here that must never be echoed.
    [ordered]@{{ error = $_.Exception.Message }} | ConvertTo-Json -Compress
}}
"#
    )
}

fn ps_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Encode a script the way `powershell -EncodedCommand` expects: UTF-16LE,
/// then standard base64.
pub fn encode_command(script: &str) -> String {
    let utf16: Vec<u8> = script
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    base64_standard(&utf16)
}

/// Base64, written out rather than pulled in.
///
/// One encoder, twenty lines, no new dependency in a security-sensitive path.
fn base64_standard(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(triple >> 18) as usize & 0x3F] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 0x3F] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6) as usize & 0x3F] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[triple as usize & 0x3F] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_reads_the_password_from_stdin() {
        let script = collection_script("10.0.0.5", "CORP\\admin", Transport::Http);
        assert!(script.contains("[Console]::In.ReadLine()"));
    }

    #[test]
    fn the_script_never_contains_a_password() {
        // The generator is not given one, and this is the test that says so:
        // the signature takes a target and an account, and there is no third
        // parameter for a future edit to reach for.
        let script = collection_script("10.0.0.5", "CORP\\admin", Transport::Http);
        assert!(!script.to_lowercase().contains("hunter2"));
        assert!(script.contains("10.0.0.5"));
        assert!(script.contains("CORP\\admin"));
    }

    #[test]
    fn the_password_is_converted_to_a_secure_string_and_the_plain_copy_dropped() {
        let script = collection_script("10.0.0.5", "admin", Transport::Http);
        assert!(script.contains("ConvertTo-SecureString"));
        assert!(script.contains("$plain = $null"));
    }

    #[test]
    fn the_error_path_reports_the_message_without_the_invocation() {
        let script = collection_script("10.0.0.5", "admin", Transport::Http);
        assert!(script.contains("$_.Exception.Message"));
        // A full error record would carry the command line that produced it.
        assert!(!script.contains("$_ | ConvertTo-Json"));
        assert!(!script.contains("$_.InvocationInfo"));
    }

    #[test]
    fn the_script_collects_the_product_type() {
        let script = collection_script("10.0.0.5", "admin", Transport::Http);
        assert!(script.contains("ProductType"));
        assert!(script.contains("Win32_OperatingSystem"));
        assert!(script.contains("Win32_ComputerSystemProduct"));
        // Interfaces, for reconciling a multi-homed machine into one device.
        assert!(script.contains("Win32_NetworkAdapterConfiguration"));
    }

    #[test]
    fn a_upn_reaches_the_script_as_a_upn() {
        // The end-to-end half of the v1.9.0 account-form regression: what the
        // operator typed is what New-Object PSCredential is handed. A rewrite
        // anywhere between the dialog and here is an authentication failure on
        // the remote machine, and it would be invisible from this side.
        let credential =
            crate::discovery::windows::WindowsCredential::new("admin@corp.example", None, "pw")
                .unwrap();
        let script = collection_script("10.0.0.5", &credential.account(), Transport::Http);
        assert!(script.contains("$account = 'admin@corp.example'"));
        assert!(!script.contains("corp.example\\admin"));
    }

    #[test]
    fn a_down_level_account_reaches_the_script_unchanged_too() {
        let credential =
            crate::discovery::windows::WindowsCredential::new("CORP\\admin", None, "pw").unwrap();
        let script = collection_script("10.0.0.5", &credential.account(), Transport::Http);
        assert!(script.contains("$account = 'CORP\\admin'"));
    }

    #[test]
    fn single_quotes_in_an_account_name_cannot_break_out_of_the_string() {
        let script = collection_script("10.0.0.5", "o'brien", Transport::Http);
        assert!(script.contains("$account = 'o''brien'"));
    }

    #[test]
    fn the_http_transport_targets_5985_without_ssl() {
        let script = collection_script("10.0.0.5", "admin", Transport::Http);
        assert!(script.contains("New-CimSessionOption -Protocol Wsman\n"));
        assert!(!script.contains("-UseSsl"));
        assert!(script.contains("-Port 5985"));
    }

    #[test]
    fn the_https_transport_targets_5986_with_ssl() {
        let script = collection_script("10.0.0.5", "admin", Transport::Https);
        assert!(script.contains("New-CimSessionOption -Protocol Wsman -UseSsl"));
        assert!(script.contains("-Port 5986"));
    }

    #[test]
    fn certificate_validation_is_never_switched_off() {
        // Turning these on would mean handing a domain credential to whoever
        // answers the connection. A scanner that does that quietly in order to
        // look like it works is a credential-harvesting opportunity.
        for transport in [Transport::Http, Transport::Https] {
            let script = collection_script("10.0.0.5", "admin", transport);
            assert!(!script.contains("SkipCACheck"));
            assert!(!script.contains("SkipCNCheck"));
            assert!(!script.contains("SkipRevocationCheck"));
        }
    }

    #[test]
    fn a_hostname_target_reaches_the_script_for_kerberos() {
        // Connecting by name lets Kerberos find an SPN; by address it cannot,
        // and the attempt falls back to NTLM or is refused outright.
        let script = collection_script("APP-01.corp.example", "admin", Transport::Http);
        assert!(script.contains("$target = 'APP-01.corp.example'"));
    }

    #[test]
    fn base64_matches_the_known_answers() {
        assert_eq!(base64_standard(b""), "");
        assert_eq!(base64_standard(b"f"), "Zg==");
        assert_eq!(base64_standard(b"fo"), "Zm8=");
        assert_eq!(base64_standard(b"foo"), "Zm9v");
        assert_eq!(base64_standard(b"foob"), "Zm9vYg==");
        assert_eq!(base64_standard(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_standard(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn encoding_produces_utf16le_base64_powershell_can_read() {
        // "hi" in UTF-16LE is 68 00 69 00, which base64s to aABpAA==.
        assert_eq!(encode_command("hi"), "aABpAA==");
    }
}
