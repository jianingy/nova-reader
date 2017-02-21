#[macro_use] extern crate clap;
#[macro_use] extern crate error_chain;
#[macro_use] extern crate log;

extern crate env_logger;
extern crate serial;

mod errors;

use clap::{App, ArgMatches};
use errors::*;
use errors::ErrorKind::*;
use env_logger::LogBuilder;
use log::{LogRecord, LogLevel, LogLevelFilter};
use serial::prelude::*;
use std::io::prelude::*;
use std::thread::sleep;
use std::time::Duration;

#[derive(Debug)]
pub struct AirQualityRecord {
    pub pm025_std: i32, // PM ug/m3 from particles with diameter <= 2.5 um.
    pub pm100_std: i32, // PM ug/m3 from particles with diameter <= 10. um.
}


pub struct NovaSensor {
    uart: serial::posix::TTYPort
}

impl NovaSensor {

    pub fn open(uart_port: &str, timeout: Duration) -> Result<Self> {
        let mut uart = serial::open(uart_port)?;
        uart.reconfigure(&|us| {
            us.set_baud_rate(serial::Baud9600)?;
            us.set_char_size(serial::Bits8);
            us.set_parity(serial::ParityNone);
            us.set_stop_bits(serial::Stop1);
            us.set_flow_control(serial::FlowNone);
            Ok(())
        })?;
        uart.set_timeout(timeout)?;
        Ok(NovaSensor { uart: uart })
    }

    pub fn read(&mut self) -> Result<AirQualityRecord> {
        let bs = self.read_from_device()?;
        self.translate(&bs)
    }

    fn read_from_device(&mut self) -> Result<Vec<u8>> {
        let mut hd = [0u8; 2];

        // searching for header '0xaa'
        for _ in 0..64 {
            self.uart.read_exact(&mut hd[0..1])?;
            if hd[0] == 0xaa { break };
        }

        if hd[0] != 0xaa {
            return Err(InvalidPacketData("cannot find start byte".to_string())
                       .into())
        }

        // searching for header '0xc0'
        for _ in 0..64 {
            self.uart.read_exact(&mut hd[1..2])?;
            if hd[1] == 0xc0 { break };
        }

        if hd[1] != 0xc0 {
            return Err(InvalidPacketData("cannot find command byte".to_string())
                       .into())
        }

        // read packet, ("bs_len - 2" skips checksum bytes)
        let mut bs = vec![0u8; 6];
        self.uart.read_exact(&mut bs)?;

        // read checksum
        let mut cs = [0u8; 1];
        self.uart.read_exact(&mut cs)?;

        let calc = (bs.iter().map(|x| *x as u16).sum::<u16>() & 0xff) as u8;
        if calc != cs[0] {
            // checksum failed
            return Err(InvalidPacketData("checksum failed".to_string())
                       .into())
        }

        Ok(bs)
    }

    fn translate(&self, bs: &[u8]) -> Result<AirQualityRecord> {
        let vals = bs.chunks(2)
            .map(|b| (((b[1] as u16) << 8) + b[0] as u16) as i32)
            .collect::<Vec<i32>>();
        Ok(AirQualityRecord {
            pm025_std: vals[0],
            pm100_std: vals[1],
        })
    }
}


fn main() {
    // setup command-line options parser
    let option_yaml = load_yaml!("options.yml");
    let matches = App::from_yaml(option_yaml).get_matches();

    // initialize logging
    LogBuilder::new()
        .format(|record: &LogRecord| {
            format!("[{}] {}",
                    match record.level() {
                        LogLevel::Error => "!",
                        LogLevel::Warn => "*",
                        LogLevel::Info => "+",
                        LogLevel::Debug => "#",
                        LogLevel::Trace => "~",
                    },
                    record.args())
        })
        .filter(None,
                match matches.occurrences_of("verbose") {
                    n if n > 2 => LogLevelFilter::Trace,
                    n if n == 2 => LogLevelFilter::Debug,
                    n if n == 1 => LogLevelFilter::Info,
                    _ => LogLevelFilter::Warn,
                })
        .init().unwrap();

    match run_monitor(&matches) {
        Err(Error(x, _)) => println!("error: {}", x),
        Ok(_) => ()
    }
}

fn run_monitor(opts: &ArgMatches) -> Result<()> {
    let port = opts.value_of("port").unwrap_or("/dev/ttyUSB0");
    let timeout = Duration::from_secs(3);
    let interval = value_t!(opts, "interval", u64)?;
    let mut device = NovaSensor::open(port, timeout)?;
    info!("warming device for 5 secs ...");
    sleep(Duration::from_secs(5));
    loop {
        let record = match device.read() {
            Ok(x) => x,
            Err(x) => {
                warn!("retry next time due to {:?}", x);
                continue;
            }
        };
        println!("PM2.5 = {}, PM10 = {}", record.pm025_std, record.pm100_std);
        sleep(Duration::from_secs(interval));
    }
}

