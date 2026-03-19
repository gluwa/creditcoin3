pub struct Roots {
    next: attestor_primitives::Height,
    rx: std::sync::mpsc::Receiver<std::task::Poll<()>>,
}

impl Roots {
    pub fn new(
        start_height: attestor_primitives::Height,
    ) -> (std::sync::mpsc::Sender<std::task::Poll<()>>, Self) {
        let (tx, rx) = std::sync::mpsc::channel();
        (
            tx,
            Self {
                next: start_height,
                rx,
            },
        )
    }
}

impl futures::Stream for Roots {
    type Item = stream_util::RootInfo;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match self.rx.recv() {
            Ok(std::task::Poll::Ready(_)) => {
                let next = self.next;
                self.next += 1;
                std::task::Poll::Ready(Some(stream_util::RootInfo {
                    height: next,
                    ..Default::default()
                }))
            }
            Ok(std::task::Poll::Pending) => std::task::Poll::Pending,
            Err(_) => std::task::Poll::Ready(None),
        }
    }
}

pub struct Tip {
    next: attestor_primitives::Height,
    rx: std::sync::mpsc::Receiver<std::task::Poll<()>>,
}

impl Tip {
    pub fn new(
        start_height: attestor_primitives::Height,
    ) -> (std::sync::mpsc::Sender<std::task::Poll<()>>, Self) {
        let (tx, rx) = std::sync::mpsc::channel();
        (
            tx,
            Self {
                next: start_height,
                rx,
            },
        )
    }
}

impl futures::Stream for Tip {
    type Item = attestor_primitives::Height;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match self.rx.recv() {
            Ok(std::task::Poll::Ready(_)) => {
                let next = self.next;
                self.next += 1;
                std::task::Poll::Ready(Some(next))
            }
            Ok(std::task::Poll::Pending) => std::task::Poll::Pending,
            Err(_) => std::task::Poll::Ready(None),
        }
    }
}
