use rppal::gpio::Gpio;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::prelude::*;
use std::os::unix::net::UnixStream;
use std::string::ToString;

#[derive(Deserialize, Debug)]
enum FanTest {
    Init,
    Test1,
    Test2,
    Stop,
}

#[derive(Deserialize, Serialize, Debug)]
struct IpConfig {
    dev: String,
    dhcp: bool,
    ip: String,
    gateway: String,
    dns: Vec<String>,

    ip_dhcp: Vec<String>,
    dns_dhcp: Vec<String>,
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
}

#[repr(u32)]
#[derive(Serialize, Debug)]
enum Error {
    SerialNotFound,
    CpuTempNotFound,
    FanTestServiceStart,
    FanTestServiceStop,
    FanTestGpio,
    FanTestPin26,
    FanTestPin27,
    FanTestSetSpeed,
    Hostnamectl,
    HostnameParse,
    HostsRead,
    HostsWrite,
    MacAddress,
    Nmcli,
    NmcliAddr,
    NmcliDhcp,
    NmcliDns,
    NmcliGateway,
    NmcliMethod,
    NmcliParse,
    NmcliUp,
    NmcliUtf8,
    NtpFallbackFile,
    NtpFallbackMalformed,
    NtpFallbackRead,
    NtpFallbackRm,
    NtpFallbackWrite,
    NtpServerFile,
    NtpServerMalformed,
    NtpServerRead,
    NtpServerRm,
    NtpServerWrite,
    Reboot,
    Rtc,
    RtsRestart,
    Spawn,
    Timedatectl,
    TimedatectlParse,
    TimesyncdRestart,
    WatchdogWrite,
}

pub fn client(mut stream: UnixStream) {
    loop {
        let cmd: Command = match serde_xml_rs::de::from_reader(&mut stream) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{e:?}");
                let _ = stream.shutdown(std::net::Shutdown::Both);
                break;
            }
        };
        let res = perform_operation(&cmd);
        let Ok(()) = (match res {
            Ok(t) => serde_xml_rs::ser::to_writer(&mut stream, &t),
            Err(e) => serde_xml_rs::ser::to_writer(&mut stream, &e),
        }) else {
            break;
        };
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

fn perform_operation(cmd: &Command) -> Result<Response, Error> {
    match cmd {
        Command::CpuSerialGet => {
            let cpuinfo = File::open("/proc/cpuinfo").or(Err(Error::SerialNotFound))?;
            let cpuinfo = std::io::BufReader::new(cpuinfo);
            cpuinfo
                .lines()
                .find_map(|l| {
                    Some(Response::CpuSerial(
                        l.ok()?
                            .strip_prefix("Serial:")?
                            .trim_start_matches([' ', '\t', ':'])
                            .to_string(),
                    ))
                })
                .ok_or(Error::SerialNotFound)
        }
        Command::CpuTempRead => Ok(Response::CpuTemp(
            std::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp")
                .or(Err(Error::CpuTempNotFound))?
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
                    "IP4.ADDRESS,ipv4.addresses,IP4.GATEWAY,IP4.DNS,ipv4.dns,ipv4.method",
                    "con",
                    "show",
                ])
                .arg(dev)
                .output()
                .or(Err(Error::Spawn))?;
            if !output.status.success() {
                return Err(Error::Nmcli);
            }
            let conf = String::from_utf8(output.stdout).or(Err(Error::NmcliUtf8))?;

            let ip4_static = conf
                .lines()
                .find_map(|l| l.strip_prefix("ipv4.addresses:"))
                .ok_or(Error::NmcliParse)?
                .to_string();

            let ip4_dhcp = conf
                .lines()
                .filter(|l| l.starts_with("IP4.ADDRESS["))
                .filter_map(|l| l.split_once(':').map(|(_, addr)| addr.to_string()))
                .filter(|a| *a != ip4_static)
                .collect::<Vec<_>>();

            let gateway = conf
                .lines()
                .find_map(|l| l.strip_prefix("IP4.GATEWAY:"))
                .ok_or(Error::NmcliParse)?
                .to_string();

            let dns_static = conf
                .lines()
                .find_map(|l| l.strip_prefix("ipv4.dns:"))
                .ok_or(Error::NmcliParse)?
                .split(',')
                .collect::<Vec<_>>();

            let dns_dhcp = conf
                .lines()
                .filter(|l| l.starts_with("IP4.DNS["))
                .filter_map(|l| l.split_once(':').map(|(_, addr)| addr))
                .filter(|a| !dns_static.contains(a))
                .map(ToString::to_string)
                .collect::<Vec<_>>();

            let dhcp = conf
                .lines()
                .find_map(|l| l.strip_prefix("ipv4.method:"))
                .ok_or(Error::NmcliParse)?
                == "auto";

            Ok(Response::IpConfig(IpConfig {
                dev: dev.clone(),
                dhcp,
                ip: ip4_static,
                gateway,
                dns: dns_static.iter().map(ToString::to_string).collect(),
                ip_dhcp: ip4_dhcp,
                dns_dhcp,
            }))
        }
        Command::IpConfigSet(conf) => {
            let dev = ["con", "mod", &conf.dev];
            if conf.dhcp {
                if !std::process::Command::new("nmcli")
                    .args(dev)
                    .args(["ipv4.method", "auto"])
                    .args(["ipv4.addresses", &conf.ip])
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
                        .arg(&conf.ip)
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
                ntp: String::from_utf8(output.stdout).or(Err(Error::TimedatectlParse))? == "yes",
                server: if fs::exists("/etc/systemd/timesyncd.conf.d/server.conf")
                    .or(Err(Error::NtpServerFile))?
                {
                    fs::read_to_string("/etc/systemd/timesyncd.conf.d/server.conf")
                        .or(Err(Error::NtpServerRead))?
                        .strip_prefix("NTP=")
                        .ok_or(Error::NtpServerMalformed)?
                        .to_string()
                } else {
                    String::new()
                },
                fallback: if fs::exists("/etc/systemd/timesyncd.conf.d/fallback.conf")
                    .or(Err(Error::NtpFallbackFile))?
                {
                    fs::read_to_string("/etc/systemd/timesyncd.conf.d/fallback.conf")
                        .or(Err(Error::NtpFallbackRead))?
                        .strip_prefix("FallbackNTP=")
                        .ok_or(Error::NtpFallbackMalformed)?
                        .split(',')
                        .map(ToString::to_string)
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
                _ => fs::write("/etc/systemd/timesyncd.conf.d/server.conf", &conf.server)
                    .or(Err(Error::NtpServerWrite))?,
            }
            if conf.fallback.contains(&"-".to_string()) {
                if fs::exists("/etc/systemd/timesyncd.conf.d/server.conf")
                    .or(Err(Error::NtpFallbackFile))?
                {
                    fs::remove_file("/etc/systemd/timesyncd.conf.d/server.conf")
                        .or(Err(Error::NtpFallbackRm))?;
                }
            } else if !conf.fallback.is_empty() {
                fs::write(
                    "/etc/systemd/timesyncd.conf.d/server.conf",
                    conf.fallback.join(" "),
                )
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
