//! The range a `read-range` asks for, from flags on the command line or in
//! the shell (#1532).

use bacnet_client::log_reader::LogCursor;
use bacnet_services::read_range::RangeSpec;
use bacnet_types::calendar::SpecificDate;
use bacnet_types::primitives::{Date, Time};

/// Items a range reads when `--count` is left out.
const DEFAULT_COUNT: i16 = 100;

/// Where a `read-range` starts, how many items it reads, and whether it
/// pages through the whole log.
#[derive(Debug, Clone, Default, PartialEq, Eq, clap::Args)]
pub struct RangeOptions {
    /// Read by position, from this one-based index.
    #[arg(long, conflicts_with_all = ["sequence", "time"])]
    pub position: Option<u64>,
    /// Read by sequence number, from this record.
    #[arg(long, conflicts_with = "time")]
    pub sequence: Option<u64>,
    /// Read the records logged after this time, in the device's local time:
    /// YYYY-MM-DDTHH:MM[:SS[.hh]].
    #[arg(long, value_parser = parse_reference_time)]
    pub time: Option<(Date, Time)>,
    /// Items to read (default 100); negative reads backward from the
    /// reference. With --all, the page size.
    #[arg(long, allow_negative_numbers = true)]
    pub count: Option<i16>,
    /// Page through the log from the oldest record, or from --position,
    /// --sequence or --time, until the newest.
    #[arg(long)]
    pub all: bool,
    /// Refuse an answer that breaks a ReadRange rule, instead of showing it
    /// with the rules it broke.
    #[arg(long, conflicts_with = "all")]
    pub strict: bool,
}

impl RangeOptions {
    fn count(&self) -> Result<i16, String> {
        match self.count.unwrap_or(DEFAULT_COUNT) {
            0 => Err("--count may not be zero".into()),
            count => Ok(count),
        }
    }

    /// The range of a single read: none (every item) when no start is given.
    pub fn spec(&self) -> Result<Option<RangeSpec>, String> {
        let count = || self.count().map(i32::from);
        Ok(match (self.position, self.sequence, self.time) {
            (Some(reference_index), _, _) => Some(RangeSpec::ByPosition {
                reference_index,
                count: count()?,
            }),
            (_, Some(reference_seq), _) => Some(RangeSpec::BySequenceNumber {
                reference_seq,
                count: count()?,
            }),
            (_, _, Some(reference_time)) => Some(RangeSpec::ByTime {
                reference_time,
                count: count()?,
            }),
            (None, None, None) if self.count.is_some() => {
                return Err("--count needs --position, --sequence, --time or --all".into())
            }
            (None, None, None) => None,
        })
    }

    /// The cursor and page size of `--all`.
    pub fn pages(&self) -> Result<(LogCursor, u16), String> {
        let cursor = match (self.position, self.sequence, self.time) {
            (Some(position), _, _) => LogCursor::Position(position),
            (_, Some(sequence), _) => LogCursor::Sequence(sequence),
            (_, _, Some((date, time))) => LogCursor::Time(date, time),
            (None, None, None) => LogCursor::Oldest,
        };
        let page_size = u16::try_from(self.count()?)
            .map_err(|_| "--all reads forward: --count must be positive".to_string())?;
        Ok((cursor, page_size))
    }

    /// Take the options out of a shell command's words, returning the rest.
    pub fn from_words(words: &[String]) -> Result<(Self, Vec<String>), String> {
        let mut options = Self::default();
        let mut rest = Vec::new();
        let mut words = words.iter();
        while let Some(word) = words.next() {
            let mut value = |flag: &str| {
                words
                    .next()
                    .cloned()
                    .ok_or_else(|| format!("{flag} needs a value"))
            };
            let reference = |flag: &str, text: String| {
                text.parse::<u64>()
                    .map_err(|_| format!("{flag} needs a number, got '{text}'"))
            };
            match word.as_str() {
                "--position" => {
                    options.position = Some(reference("--position", value("--position")?)?)
                }
                "--sequence" => {
                    options.sequence = Some(reference("--sequence", value("--sequence")?)?)
                }
                "--time" => options.time = Some(parse_reference_time(&value("--time")?)?),
                "--count" => {
                    let text = value("--count")?;
                    options.count = Some(
                        text.parse()
                            .map_err(|_| format!("--count needs -32768..=32767, got '{text}'"))?,
                    );
                }
                "--all" => options.all = true,
                "--strict" => options.strict = true,
                flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
                _ => rest.push(word.clone()),
            }
        }
        let starts = [
            options.position.is_some(),
            options.sequence.is_some(),
            options.time.is_some(),
        ];
        if starts.iter().filter(|start| **start).count() > 1 {
            return Err("give one of --position, --sequence and --time".into());
        }
        if options.strict && options.all {
            return Err("--strict takes one range; --all reads leniently".into());
        }
        Ok((options, rest))
    }
}

fn number<T: std::str::FromStr>(text: &str, what: &str) -> Result<T, String> {
    text.parse()
        .map_err(|_| format!("invalid {what} '{text}' in --time"))
}

/// Parse `YYYY-MM-DDTHH:MM[:SS[.hh]]` (a space may stand for the `T`) as a
/// specific local date and time, with the weekday the date falls on.
pub fn parse_reference_time(text: &str) -> Result<(Date, Time), String> {
    let usage = || format!("--time must be YYYY-MM-DDTHH:MM[:SS[.hh]], got '{text}'");
    let (date, time) = text.split_once(['T', ' ']).ok_or_else(usage)?;
    let [year, month, day] =
        <[&str; 3]>::try_from(date.split('-').collect::<Vec<_>>()).map_err(|_| usage())?;
    let day = SpecificDate::new(
        number(year, "year")?,
        number(month, "month")?,
        number(day, "day")?,
    )
    .ok_or_else(|| format!("--time date '{date}' is not a day in 1900..=2154"))?;
    let (clock, hundredths) = match time.split_once('.') {
        Some((clock, fraction)) if fraction.len() == 2 => (clock, number(fraction, "hundredths")?),
        Some(_) => return Err(usage()),
        None => (time, 0),
    };
    let parts: Vec<&str> = clock.split(':').collect();
    let (hour, minute, second) = match parts.as_slice() {
        [hour, minute] => (number(hour, "hour")?, number(minute, "minute")?, 0),
        [hour, minute, second] => (
            number(hour, "hour")?,
            number(minute, "minute")?,
            number(second, "second")?,
        ),
        _ => return Err(usage()),
    };
    let time = Time {
        hour,
        minute,
        second,
        hundredths,
    };
    if !time.is_specific() {
        return Err(format!("--time '{text}' is not a time of day"));
    }
    Ok((day.to_date(), time))
}

#[cfg(test)]
#[path = "read_range_options_tests.rs"]
mod tests;
