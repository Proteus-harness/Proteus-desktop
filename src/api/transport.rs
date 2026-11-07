//! Cap each blocking read, including long-lived SSE, without giving a healthy
//! stream a total lifetime. Keep ureq's standard proxy, TCP and TLS chain.
use std::time::Duration;
use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, DefaultConnector, NextTimeout, Transport,
};

pub(super) fn agent(config: ureq::config::Config, idle: Duration) -> ureq::Agent {
    ureq::Agent::with_parts(
        config,
        DefaultConnector::default().chain(IdleConnector(idle)),
        DefaultResolver::default(),
    )
}

#[derive(Debug)]
struct IdleConnector(Duration);

impl Connector<Box<dyn Transport>> for IdleConnector {
    type Out = IdleTransport;
    fn connect(
        &self,
        _: &ConnectionDetails,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        Ok(chained.map(|inner| IdleTransport {
            inner,
            idle: self.0,
        }))
    }
}

#[derive(Debug)]
struct IdleTransport {
    inner: Box<dyn Transport>,
    idle: Duration,
}

impl Transport for IdleTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }
    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        self.inner.transmit_output(amount, timeout)
    }
    fn await_input(&mut self, mut timeout: NextTimeout) -> Result<bool, ureq::Error> {
        let idle = ureq::unversioned::transport::time::Duration::Exact(self.idle);
        if timeout.after > idle {
            timeout.after = idle;
            timeout.reason = ureq::Timeout::RecvBody;
        }
        self.inner.await_input(timeout)
    }
    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }
    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}
