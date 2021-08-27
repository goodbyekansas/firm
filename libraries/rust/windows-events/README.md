# Windows Events

A simple [Rust slog](https://docs.rs/slog/latest/slog/) backend to send messages to the [Windows event log](https://docs.microsoft.com/en-us/windows/desktop/eventlog/event-logging).

## Features

* Writes Rust log messages to the Windows event log using the
  [RegisterEventSourceW](https://docs.microsoft.com/en-us/windows/desktop/api/Winbase/nf-winbase-registereventsourcew)
  and [ReportEventW](https://docs.microsoft.com/en-us/windows/desktop/api/winbase/nf-winbase-reporteventw) APIs.
* Provides utility functions to register/deregister your
  [event source](https://docs.microsoft.com/en-us/windows/desktop/eventlog/event-sources) in the Windows registry.
* Embeds a small (120-byte) message resource library containing the
  necessary log message templates in your executable.

The five Rust log levels are mapped to Windows [event types](https://docs.microsoft.com/en-us/windows/desktop/eventlog/event-types) as follows:

| Rust Log Level | Windows Event Type | Windows Event Id |
| -------------- | ------------------ | ---------------- |
| Error          | Error              | 1                |
| Warn           | Warning            | 2                |
| Info           | Informational      | 3                |
| Debug          | Informational      | 4                |
| Trace          | Informational      | 5                |


### Register log source with Windows

Register the log source in the Windows registry:
```rust
windows_events::try_register("Example Log").unwrap();
```
This usually requires `Administrator` permission so this is usually done during
installation time.

If your MSI installer (or similar) registers your event sources you should not call this.


### Log events

```
use slog::{info, o, Drain, Logger};
use windows_events::WinLogger;

let win_logger = WinLogger::try_new("Example Log").unwrap();
let logger = Logger::root(win_logger.ignore_res(), o!());

info!(logger, "Hello, Event Log");
```

### Deregister log source

Deregister the log source: 
```
windows_events::try_deregister("Example Log").unwrap();
```
This is usually done during program uninstall. If your MSI 
installer (or similar) deregisters your event sources you should not call this.


## License

Licensed under either of

* Apache License, Version 2.0 (LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0)
* MIT license (LICENSE-MIT or http://opensource.org/licenses/MIT)

at your option.


## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted 
for inclusion in the work by you, as defined in the Apache-2.0 license, shall 
be dual licensed as above, without any additional terms or conditions.
