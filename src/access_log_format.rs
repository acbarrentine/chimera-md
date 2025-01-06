use std::fmt;
use axum::http::Version;
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

#[allow(clippy::too_many_arguments)]
pub fn log_access(
    status: u16,
    method: &str,
    bytes: &str,
    version: Version,
    uri: &str,
    addr: &str,
    user_agent: Option<String>,
    referer: Option<String>,
) {
    // "Combined" log format. Example:
    // 127.0.0.1 - - [05/Feb/2012:17:11:55 +0000] "GET / HTTP/1.1" 200 140 "-" "Mozilla/5.0 (Windows NT 6.1; WOW64) AppleWebKit/535.19 (KHTML, like Gecko) Chrome/18.0.1025.5 Safari/535.19"
    let now = Local::now();
    info!(target: "access_log", "{addr} - - [{}] \"{method} {uri} {version:?}\" {status} {bytes} \"{}\" \"{}\"",
        now.format("%d/%b/%Y:%H:%M:%S %z"),
        optional(referer),
        optional(user_agent),
    );
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
