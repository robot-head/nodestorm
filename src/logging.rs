use tracing::Subscriber;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

const DEFAULT_FILTER: &str = "warn,nodestorm=info";

fn compact_subscriber<W>(writer: W, filter: EnvFilter) -> impl Subscriber + Send + Sync
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    tracing_subscriber::fmt()
        .compact()
        .with_target(false)
        .with_env_filter(filter)
        .with_writer(writer)
        .finish()
}

pub fn init() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    tracing::subscriber::set_global_default(compact_subscriber(std::io::stderr, filter))
        .expect("global tracing subscriber already set");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    struct BufferGuard(Buffer);

    impl Write for BufferGuard {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Buffer {
        type Writer = BufferGuard;

        fn make_writer(&'a self) -> Self::Writer {
            BufferGuard(self.clone())
        }
    }

    #[test]
    fn default_output_is_compact_and_hides_dependency_info() {
        let output = Buffer::default();
        let subscriber = compact_subscriber(
            output.clone(),
            tracing_subscriber::EnvFilter::new(DEFAULT_FILTER),
        );

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "nodestorm", "app ready");
            tracing::info!(
                target: "rmcp::service",
                peer_info = "many fields",
                "initialized"
            );
            tracing::warn!(target: "rmcp::service", "transport warning");
        });

        let rendered = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
        assert!(rendered.contains("app ready"));
        assert!(rendered.contains("transport warning"));
        assert!(!rendered.contains("many fields"));
        assert!(!rendered.contains("rmcp::service"));
        assert!(!rendered.contains("nodestorm:"));
    }
}
