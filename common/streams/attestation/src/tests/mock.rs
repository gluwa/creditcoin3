//! Mock implementations of the [eth root stream] and the [eth tip stream] for use in testing.
//!
//! [eth root stream]: stream_eth::StreamRoots
//! [eth tip stream]: stream_eth::StreamTip

#[derive(Clone)]
pub struct RootSender {
    tx: PollSend<std::task::Poll<()>>,
}

#[derive(Clone)]
pub struct RootReceiver {
    next: attestor_primitives::Height,
    rx: PollRecv<std::task::Poll<()>>,
}

pub fn roots(start_height: attestor_primitives::Height) -> (RootSender, RootReceiver) {
    let (tx, rx) = channel();
    let next = start_height;

    (RootSender { tx }, RootReceiver { next, rx })
}

impl RootSender {
    #[cfg(feature = "simulation")]
    pub fn send(&mut self, poll: std::task::Poll<()>) {
        self.tx.send(poll);
    }

    pub fn send_ready(&mut self) {
        self.tx.send(std::task::Poll::Ready(()));
    }

    #[allow(unused)]
    pub fn send_pending(&mut self) {
        self.tx.send(std::task::Poll::Pending);
    }
}

impl futures::Stream for RootReceiver {
    type Item = stream_util::RootInfo;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::ready!(self.rx.poll(cx)).map(|_| {
            let next = self.next;
            self.next += 1;
            Some(stream_util::RootInfo {
                height: next,
                ..Default::default()
            })
        })
    }
}

impl stream_util::ChainData<stream_util::RootInfo> for RootReceiver {
    async fn reset(&self, info: stream_util::AttestationInfo) -> Self {
        let mut receiver = self.clone();
        receiver.next = info.height;
        receiver
    }
}

#[derive(Clone)]
pub struct TipSender {
    tx: PollSend<std::task::Poll<()>>,
}

#[derive(Clone)]
pub struct TipReceiver {
    next: attestor_primitives::Height,
    rx: PollRecv<std::task::Poll<()>>,
}

pub fn tip(start_height: attestor_primitives::Height) -> (TipSender, TipReceiver) {
    let (tx, rx) = channel();
    let next = start_height;

    (TipSender { tx }, TipReceiver { next, rx })
}

impl TipSender {
    #[cfg(feature = "simulation")]
    pub fn send(&mut self, poll: std::task::Poll<()>) {
        self.tx.send(poll);
    }

    pub fn send_ready(&mut self) {
        self.tx.send(std::task::Poll::Ready(()));
    }

    #[allow(unused)]
    pub async fn send_pending(&mut self) {
        self.tx.send(std::task::Poll::Pending);
    }
}

impl futures::Stream for TipReceiver {
    type Item = attestor_primitives::Height;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::ready!(self.rx.poll(cx)).map(|_| {
            let next = self.next;
            self.next += 1;
            Some(next)
        })
    }
}

impl stream_util::ChainData<attestor_primitives::Height> for TipReceiver {
    async fn reset(&self, info: stream_util::AttestationInfo) -> Self {
        let mut receiver = self.clone();
        receiver.next = info.height;
        receiver
    }
}

#[derive(Clone)]
struct PollSend<T>(std::sync::Arc<std::sync::Mutex<PollQueue<T>>>);
#[derive(Clone)]
struct PollRecv<T>(std::sync::Arc<std::sync::Mutex<PollQueue<T>>>);

struct PollQueue<T> {
    queue: std::collections::VecDeque<T>,
    waker: Option<std::task::Waker>,
}

fn channel<T>() -> (PollSend<T>, PollRecv<T>) {
    let queue_send = std::sync::Arc::new(std::sync::Mutex::new(PollQueue::new()));
    let queue_recv = std::sync::Arc::clone(&queue_send);

    (PollSend(queue_send), PollRecv(queue_recv))
}

impl<T> PollSend<T> {
    fn send(&self, elem: T) {
        self.0.lock().unwrap().push(elem);
    }
}

impl<T> PollRecv<T> {
    fn poll(&self, cx: &mut std::task::Context<'_>) -> std::task::Poll<T> {
        self.0.lock().unwrap().poll(cx)
    }
}

impl<T> PollQueue<T> {
    fn new() -> Self {
        Self {
            queue: std::collections::VecDeque::new(),
            waker: None,
        }
    }

    fn push(&mut self, elem: T) {
        self.queue.push_front(elem);
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    fn poll(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<T> {
        match self.queue.pop_back() {
            Some(elem) => std::task::Poll::Ready(elem),
            None => {
                self.waker.replace(cx.waker().clone());
                std::task::Poll::Pending
            }
        }
    }
}
