use rppal::gpio::Gpio;
use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::prelude::*;
use std::os::unix::net::UnixStream;
use std::string::ToString;

#[derive(Deserialize_repr, Debug)]
#[repr(u8)]
enum FanTest {
    Init = 0,
    Test1 = 1,
    Test2 = 2,
    Stop = 3,
}

#[derive(Deserialize, Serialize, Debug)]
struct IpConfig {
    dev: String,
    dhcp: bool,
    ip_dhcp: String,
    gateway: String,

    #[serde(default)]
    dns: Vec<String>,
    #[serde(default)]
    ip: Vec<String>,
    #[serde(default)]
    dns_dhcp: Vec<String>,
}

impl TryFrom<String> for IpConfig {
    type Error = Error;
    fn try_from(conf: String) -> Result<Self, Self::Error> {
        /*        let ip4_static = conf
        .lines()
        .find_map(|l| l.strip_prefix("ipv4.addresses:"))
        .ok_or(Error::NmcliParse)?
        .to_string();*/

        let dhcp = conf
            .lines()
            .filter_map(|l| {
                l.strip_prefix("DHCP4.OPTION[")?
                    .trim_start_matches([
                        '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', ']', ':',
                    ])
                    .split_once('=')
                    .map(|(k, v)| (k.trim(), v.trim()))
            })
            .collect::<HashMap<_, _>>();

        let ip4_dhcp = dhcp.get("ip_address");

        let ip4_static = conf
            .lines()
            .filter(|l| l.starts_with("IP4.ADDRESS["))
            .filter_map(|l| l.split_once(':').map(|(_, addr)| addr.to_string()))
            .filter(|a| {
                !ip4_dhcp.is_some_and(|ip| dbg!(dbg!(dbg!(a).strip_prefix(ip)).is_some_and(|aa| dbg!(dbg!(aa).starts_with("/")))))
            })
            .collect::<Vec<_>>();

        let gateway = conf
            .lines()
            .find_map(|l| l.strip_prefix("IP4.GATEWAY:"))
            .ok_or(Error::NmcliParse)?
            .to_string();

        let dns_dhcp = dhcp
            .get("domain_name_servers")
            .unwrap_or(&"")
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.'))
            .map(&str::to_string)
            .collect::<Vec<_>>();

        let dns_static = conf
            .lines()
            .filter(|l| l.starts_with("IP4.DNS["))
            .filter_map(|l| l.split_once(':').map(|(_, addr)| addr.to_string()))
            .filter(|a| !dns_dhcp.contains(a))
            //          .map(ToString::to_string)
            .collect::<Vec<_>>();

        let method = conf
            .lines()
            .find_map(|l| l.strip_prefix("ipv4.method:"))
            .ok_or(Error::NmcliParse)?
            == "auto";

        Ok(IpConfig {
            dev: String::new(),
            dhcp: method,
            ip: ip4_static,
            gateway,
            dns: dns_static.iter().map(ToString::to_string).collect(),
            ip_dhcp: ip4_dhcp.unwrap_or(&"").to_string(),
            dns_dhcp,
        })
    }
}

#[derive(Deserialize, Serialize, Debug)]
struct NtpConfig {
    ntp: bool,
    server: String,
    fallback: Vec<String>,
}

#[derive(Deserialize, Debug)]
enum Command {
    CpuSerialGet,
    CpuTempRead,
    FanTest(FanTest),
    HostnameGet,
    HostnameSet(String),
    IpConfigGet(String),
    IpConfigSet(IpConfig),
    MacAddressGet(String),
    NtpConfigGet,
    NtpConfigSet(NtpConfig),
    OsReboot,
    RtcCheck,
    RtsRestart,
    TimezoneGet,
    TimezoneSet(String),
    WatchdogWrite(u16),
}

#[derive(Serialize, Debug)]
#[serde(rename = "Result")]
enum Response {
    CpuSerial(String),
    CpuTemp(u32),
    Hostname(String),
    IpConfig(IpConfig),
    MacAddress(String),
    NtpConfig(NtpConfig),
    Rtc(bool),
    Timezone(String),
    Success(()),
    Error(Error),
}

#[repr(u32)]
#[derive(Serialize_repr, Debug)]
enum Error {
    CpuinfoOpen = 101,
    CpuSerialNotFound = 102,

    CpuTempNotFound = 12,
    CpuTempOpen = 11,

    FanTestServiceStart = 114,
    FanTestServiceStop = 111,
    FanTestGpio = 113,
    FanTestPin26 = 116,
    FanTestPin27 = 117,
    FanTestSetSpeed = 112,

    Hostnamectl = 21,
    HostnameParse = 22,
    HostsRead = 23,
    HostsWrite = 24,

    MacAddress = 41,

    Nmcli = 31,
    NmcliAddr = 35,
    NmcliDhcp = 34,
    NmcliDns = 37,
    NmcliGateway = 36,
    NmcliMethod = 38,
    NmcliParse = 33,
    NmcliUp = 39,
    NmcliUtf8 = 32,

    NtpFallbackFile = 56,
    NtpFallbackMalformed = 58,
    NtpFallbackRead = 57,
    NtpFallbackRm = 63,
    NtpFallbackWrite = 64,
    NtpServerFile = 53,
    NtpServerMalformed = 55,
    NtpServerRead = 54,
    NtpServerRm = 61,
    NtpServerWrite = 62,

    Reboot = 3,

    Rtc = 5,

    RtsRestart = 7,

    Spawn = 1,

    Timedatectl = 51,
    TimedatectlParse = 52,
    TimesyncdRestart = 65,

    WatchdogWrite = 9,

    SyntaxError = 99,
}

pub fn client(mut stream: UnixStream) {
    loop {
        let cmd: Command = match serde_xml_rs::de::from_reader(&mut stream) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{e:?}");
                let _ =
                    serde_xml_rs::ser::to_writer(&mut stream, &Response::Error(Error::SyntaxError));
                let _ = stream.shutdown(std::net::Shutdown::Both);
                break;
            }
        };
        let res = match perform_operation(&cmd) {
            Ok(t) => t,
            Err(e) => Response::Error(e),
        };
        match serde_xml_rs::ser::to_writer(&mut stream, &res) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("{e:?}");
                break;
            }
        }
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

fn perform_operation(cmd: &Command) -> Result<Response, Error> {
    match cmd {
        Command::CpuSerialGet => {
            let cpuinfo = File::open("/proc/cpuinfo").or(Err(Error::CpuinfoOpen))?;
            let cpuinfo = std::io::BufReader::new(cpuinfo);
            cpuinfo
                .lines()
                .find_map(|l| {
                    Some(Response::CpuSerial(
                        l.ok()?
                            .strip_prefix("Serial")?
                            .trim_start_matches([' ', '\t', ':'])
                            .to_string(),
                    ))
                })
                .ok_or(Error::CpuSerialNotFound)
        }
        Command::CpuTempRead => Ok(Response::CpuTemp(
            std::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp")
                .or(Err(Error::CpuTempOpen))?
                .trim()
                .parse::<u32>()
                .or(Err(Error::CpuTempNotFound))?,
        )),
        Command::FanTest(phase) => {
            match phase {
                FanTest::Init => {
                    if !std::process::Command::new("systemctl")
                        .args(["--no-ask-password", "stop", "wp360-fan-ctrl"])
                        .status()
                        .or(Err(Error::Spawn))?
                        .success()
                    {
                        return Err(Error::FanTestServiceStop);
                    }

                    fs::write("/sys/kernel/wp360-pmuc/fan_voltage", "90")
                        .or(Err(Error::FanTestSetSpeed))?;

                    let gpio = Gpio::new().or(Err(Error::FanTestGpio))?;
                    gpio.get(26)
                        .or(Err(Error::FanTestPin26))?
                        .into_output()
                        .set_low();
                    gpio.get(27)
                        .or(Err(Error::FanTestPin27))?
                        .into_output()
                        .set_low();
                }
                FanTest::Test1 => {
                    let gpio = Gpio::new().or(Err(Error::FanTestGpio))?;
                    gpio.get(26)
                        .or(Err(Error::FanTestPin26))?
                        .into_output()
                        .set_high();
                    gpio.get(27)
                        .or(Err(Error::FanTestPin27))?
                        .into_output()
                        .set_low();
                }
                FanTest::Test2 => {
                    let gpio = Gpio::new().or(Err(Error::FanTestGpio))?;
                    gpio.get(26)
                        .or(Err(Error::FanTestPin26))?
                        .into_output()
                        .set_low();
                    gpio.get(27)
                        .or(Err(Error::FanTestPin27))?
                        .into_output()
                        .set_high();
                }
                FanTest::Stop => {
                    if !std::process::Command::new("systemctl")
                        .args(["--no-ask-password", "start", "wp360-fan-ctrl"])
                        .status()
                        .or(Err(Error::Spawn))?
                        .success()
                    {
                        return Err(Error::FanTestServiceStart);
                    }
                }
            }
            Ok(Response::Success(()))
        }
        Command::HostnameGet => {
            let output = std::process::Command::new("hostnamectl")
                .arg("hostname")
                .output()
                .or(Err(Error::Spawn))?;
            if !output.status.success() {
                return Err(Error::Hostnamectl);
            }
            Ok(Response::Hostname(
                String::from_utf8(output.stdout)
                    .or(Err(Error::HostnameParse))?
                    .trim()
                    .to_string(),
            ))
        }
        Command::HostnameSet(hostname) => {
            if !std::process::Command::new("hostnamectl")
                .args(["--no-ask-password", "hostname"])
                .arg(hostname)
                .status()
                .or(Err(Error::Spawn))?
                .success()
            {
                return Err(Error::Hostnamectl);
            }
            let hosts = fs::read_to_string("/etc/hosts").or(Err(Error::HostsRead))?;
            fs::write(
                "/etc/hosts",
                hosts
                    .lines()
                    .map(|l| {
                        if l.starts_with("127.0.1.1") {
                            format!("127.0.1.1\t\t{hostname}\n")
                        } else {
                            format!("{l}\n")
                        }
                    })
                    .collect::<String>(),
            )
            .or(Err(Error::HostsWrite))?;
            Ok(Response::Success(()))
        }
        Command::IpConfigGet(dev) => {
            let output = std::process::Command::new("nmcli")
                .args([
                    "--terse",
                    "-f",
                    "IP4.ADDRESS,ipv4.addresses,IP4.GATEWAY,IP4.DNS,ipv4.dns,ipv4.method,DHCP4.OPTION",
                    "con",
                    "show",
                ])
                .arg(dev)
                .output()
                .or(Err(Error::Spawn))?;
            if !output.status.success() {
                return Err(Error::Nmcli);
            }
            Ok(Response::IpConfig(IpConfig {
                dev: dev.clone(),
                ..String::from_utf8(output.stdout)
                    .or(Err(Error::NmcliUtf8))?
                    .try_into()?
            }))
        }
        Command::IpConfigSet(conf) => {
            let dev = ["con", "mod", &conf.dev];
            if conf.dhcp {
                if !std::process::Command::new("nmcli")
                    .args(dev)
                    .args(["ipv4.method", "auto"])
                    .args(["ipv4.addresses", &conf.ip.join(",")])
                    .args(["ipv4.gateway", &conf.gateway])
                    .arg("ipv4.dns")
                    .arg(conf.dns.join(","))
                    .status()
                    .or(Err(Error::Spawn))?
                    .success()
                {
                    return Err(Error::NmcliDhcp);
                }
            } else {
                if !conf.ip.is_empty()
                    && !std::process::Command::new("nmcli")
                        .args(dev)
                        .arg("ipv4.addresses")
                        .arg(conf.ip.join(","))
                        .status()
                        .or(Err(Error::Spawn))?
                        .success()
                {
                    return Err(Error::NmcliAddr);
                }

                if !conf.gateway.is_empty()
                    && !std::process::Command::new("nmcli")
                        .args(dev)
                        .args(["ipv4.gateway", &conf.gateway])
                        .status()
                        .or(Err(Error::Spawn))?
                        .success()
                {
                    return Err(Error::NmcliGateway);
                }

                if !conf.dns.is_empty()
                    && !std::process::Command::new("nmcli")
                        .args(dev)
                        .arg("ipv4.dns")
                        .arg(conf.dns.join(","))
                        .status()
                        .or(Err(Error::Spawn))?
                        .success()
                {
                    return Err(Error::NmcliDns);
                }

                if !std::process::Command::new("nmcli")
                    .args(dev)
                    .args(["ipv4.method", "manual"])
                    .status()
                    .or(Err(Error::Spawn))?
                    .success()
                {
                    return Err(Error::NmcliMethod);
                }
            }
            if !std::process::Command::new("nmcli")
                .args(["con", "up", &conf.dev])
                .status()
                .or(Err(Error::Spawn))?
                .success()
            {
                return Err(Error::NmcliUp);
            }
            Ok(Response::Success(()))
        }
        Command::MacAddressGet(dev) => Ok(Response::MacAddress(
            fs::read_to_string(format!("/sys/class/net/{dev}/address"))
                .or(Err(Error::MacAddress))?
                .trim()
                .to_string(),
        )),
        Command::NtpConfigGet => {
            let output = std::process::Command::new("timedatectl")
                .args(["show", "-P", "NTP"])
                .output()
                .or(Err(Error::Spawn))?;
            if !output.status.success() {
                return Err(Error::Timedatectl);
            }

            Ok(Response::NtpConfig(NtpConfig {
                ntp: String::from_utf8(output.stdout)
                    .or(Err(Error::TimedatectlParse))?
                    .trim()
                    == "yes",
                server: if fs::exists("/etc/systemd/timesyncd.conf.d/server.conf")
                    .or(Err(Error::NtpServerFile))?
                {
                    fs::read_to_string("/etc/systemd/timesyncd.conf.d/server.conf")
                        .or(Err(Error::NtpServerRead))?
                        .strip_prefix("[Time]\nNTP=")
                        .ok_or(Error::NtpServerMalformed)?
                        .trim()
                        .to_string()
                } else {
                    String::new()
                },
                fallback: if fs::exists("/etc/systemd/timesyncd.conf.d/fallback.conf")
                    .or(Err(Error::NtpFallbackFile))?
                {
                    fs::read_to_string("/etc/systemd/timesyncd.conf.d/fallback.conf")
                        .or(Err(Error::NtpFallbackRead))?
                        .strip_prefix("[Time]\nFallbackNTP=")
                        .ok_or(Error::NtpFallbackMalformed)?
                        .split(' ')
                        .map(|a| a.trim().to_string())
                        .collect()
                } else {
                    vec![String::new()]
                },
            }))
        }
        Command::NtpConfigSet(conf) => {
            match conf.server.as_str() {
                "" => {}
                "-" => {
                    if fs::exists("/etc/systemd/timesyncd.conf.d/server.conf")
                        .or(Err(Error::NtpServerFile))?
                    {
                        fs::remove_file("/etc/systemd/timesyncd.conf.d/server.conf")
                            .or(Err(Error::NtpServerRm))?;
                    }
                }
                _ => {
                    let mut server = "[Time]\nNTP=".to_string();
                    server.push_str(&conf.server);
                    fs::write("/etc/systemd/timesyncd.conf.d/server.conf", &server)
                        .or(Err(Error::NtpServerWrite))?;
                }
            }
            if conf.fallback.contains(&"-".to_string()) {
                if fs::exists("/etc/systemd/timesyncd.conf.d/fallback.conf")
                    .or(Err(Error::NtpFallbackFile))?
                {
                    fs::remove_file("/etc/systemd/timesyncd.conf.d/fallback.conf")
                        .or(Err(Error::NtpFallbackRm))?;
                }
            } else if !conf.fallback.is_empty() {
                let mut fallback = "[Time]\nFallbackNTP=".to_string();
                fallback.push_str(&conf.fallback.join(" "));
                fs::write("/etc/systemd/timesyncd.conf.d/fallback.conf", &fallback)
                    .or(Err(Error::NtpFallbackWrite))?;
            }
            if !std::process::Command::new("timedatectl")
                .arg("set-ntp")
                .arg(conf.ntp.to_string())
                .status()
                .or(Err(Error::Spawn))?
                .success()
            {
                return Err(Error::Timedatectl);
            }
            if conf.ntp
                && !std::process::Command::new("systemctl")
                    .args(["--no-ask-password", "restart", "systemd-timesyncd"])
                    .status()
                    .or(Err(Error::Spawn))?
                    .success()
            {
                return Err(Error::TimesyncdRestart);
            }
            Ok(Response::Success(()))
        }
        Command::OsReboot => {
            if !std::process::Command::new("reboot")
                .status()
                .or(Err(Error::Spawn))?
                .success()
            {
                return Err(Error::Reboot);
            }
            Ok(Response::Success(()))
        }
        Command::RtcCheck => Ok(Response::Rtc(fs::exists("/dev/rtc").or(Err(Error::Rtc))?)),
        Command::RtsRestart => {
            if !std::process::Command::new("systemctl")
                .args(["--no-ask-password", "restart", "codesyscontrol"])
                .status()
                .or(Err(Error::Spawn))?
                .success()
            {
                return Err(Error::RtsRestart);
            }
            Ok(Response::Success(()))
        }
        Command::TimezoneGet => {
            let output = std::process::Command::new("timedatectl")
                .args(["show", "-P", "Timezone"])
                .output()
                .or(Err(Error::Spawn))?;
            if !output.status.success() {
                return Err(Error::Timedatectl);
            }
            Ok(Response::Timezone(
                String::from_utf8(output.stdout)
                    .or(Err(Error::TimedatectlParse))?
                    .trim()
                    .to_string(),
            ))
        }
        Command::TimezoneSet(tz) => {
            if !std::process::Command::new("timedatectl")
                .args(["--no-ask-password", "set-timezone", tz])
                .status()
                .or(Err(Error::Spawn))?
                .success()
            {
                return Err(Error::Timedatectl);
            }
            Ok(Response::Success(()))
        }
        Command::WatchdogWrite(count) => {
            fs::write("/sys/kernel/wp360-pmuc/watchdog", count.to_string())
                .or(Err(Error::WatchdogWrite))?;
            Ok(Response::Success(()))
        }
    }
}
