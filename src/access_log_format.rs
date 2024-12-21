use std::fmt;
use chrono::Local;
use tracing::{info, Event, Subscriber};
use tracing_subscriber::fmt::{
    format::{self, FormatEvent, FormatFields},
    FmtContext,
};
use tracing_subscriber::registry::LookupSpan;

fn optional(opt: Option<String>) -> String {
    opt.unwrap_or(String::from("-"))
}

pub fn log_access(
    status: u16,
    uri: &str,
    addr: &str,
    user_agent: Option<String>,
    referer: Option<String>,
) {
    let now = Local::now();
    info!(target: "access_log", "{addr} - - [{}] \"{status} {uri} HTTP/1.1\" {} {}",
        now.format("%d-%b-%Y:%H:%M:%S %z"),
        optional(user_agent),
        optional(referer));
}

pub struct AccessLogFormat;

impl<S, N> FormatEvent<S, N> for AccessLogFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: format::Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        // Format values from the event's's metadata:
        let metadata = event.metadata();
        if metadata.target() == "access_log" {
            ctx.field_format().format_fields(writer.by_ref(), event)?;
            writeln!(writer)
        }
        else {
            Ok(())
        }
    }
}
