pub struct RootSender {
    tx: futures::channel::mpsc::UnboundedSender<std::task::Poll<()>>,
}

pub struct RootReceiver {
    next: attestor_primitives::Height,
    rx: futures::channel::mpsc::UnboundedReceiver<std::task::Poll<()>>,
}

pub fn roots(start_height: attestor_primitives::Height) -> (RootSender, RootReceiver) {
    let (tx, rx) = futures::channel::mpsc::unbounded();
    (
        RootSender { tx },
        RootReceiver {
            next: start_height,
            rx,
        },
    )
}

impl RootSender {
    pub async fn send(&mut self, poll: std::task::Poll<()>) {
        use futures::SinkExt as _;
        self.tx.send(poll).await.unwrap();
    }

    pub async fn send_ready(&mut self) {
        use futures::SinkExt as _;
        self.tx.send(std::task::Poll::Ready(())).await.unwrap();
    }

    pub async fn send_pending(&mut self) {
        use futures::SinkExt as _;
        self.tx.send(std::task::Poll::Pending).await.unwrap();
    }
}

impl futures::Stream for RootReceiver {
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

pub struct TipSender {
    tx: futures::channel::mpsc::UnboundedSender<std::task::Poll<()>>,
}

pub struct TipReceiver {
    next: attestor_primitives::Height,
    rx: futures::channel::mpsc::UnboundedReceiver<std::task::Poll<()>>,
}

pub fn tip(start_height: attestor_primitives::Height) -> (TipSender, TipReceiver) {
    let (tx, rx) = futures::channel::mpsc::unbounded();
    (
        TipSender { tx },
        TipReceiver {
            next: start_height,
            rx,
        },
    )
}

impl TipSender {
    pub async fn send(&mut self, poll: std::task::Poll<()>) {
        use futures::SinkExt as _;
        self.tx.send(poll).await.unwrap();
    }

    pub async fn send_ready(&mut self) {
        use futures::SinkExt as _;
        self.tx.send(std::task::Poll::Ready(())).await.unwrap();
    }

    pub async fn send_pending(&mut self) {
        use futures::SinkExt as _;
        self.tx.send(std::task::Poll::Pending).await.unwrap();
    }
}

impl futures::Stream for TipReceiver {
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
