pub struct Roots {
    next: attestor_primitives::Height,
    rx: futures::channel::mpsc::UnboundedReceiver<std::task::Poll<()>>,
}

impl Roots {
    pub fn new(
        start_height: attestor_primitives::Height,
    ) -> (
        futures::channel::mpsc::UnboundedSender<std::task::Poll<()>>,
        Self,
    ) {
        let (tx, rx) = futures::channel::mpsc::unbounded();
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
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use futures::StreamExt as _;

        match std::task::ready!(self.rx.poll_next_unpin(cx)) {
            Some(std::task::Poll::Ready(_)) => {
                let next = self.next;
                self.next += 1;
                std::task::Poll::Ready(Some(stream_util::RootInfo {
                    height: next,
                    ..Default::default()
                }))
            }
            Some(std::task::Poll::Pending) => std::task::Poll::Pending,
            None => std::task::Poll::Ready(None),
        }
    }
}

pub struct Tip {
    next: attestor_primitives::Height,
    rx: futures::channel::mpsc::UnboundedReceiver<std::task::Poll<()>>,
}

impl Tip {
    pub fn new(
        start_height: attestor_primitives::Height,
    ) -> (
        futures::channel::mpsc::UnboundedSender<std::task::Poll<()>>,
        Self,
    ) {
        let (tx, rx) = futures::channel::mpsc::unbounded();
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
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use futures::StreamExt as _;

        match std::task::ready!(self.rx.poll_next_unpin(cx)) {
            Some(std::task::Poll::Ready(_)) => {
                let next = self.next;
                self.next += 1;
                std::task::Poll::Ready(Some(next))
            }
            Some(std::task::Poll::Pending) => std::task::Poll::Pending,
            None => std::task::Poll::Ready(None),
        }
    }
}
