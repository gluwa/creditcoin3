pub(crate) struct OptionFuture<Output = ()>(
    Option<std::pin::Pin<Box<dyn std::future::Future<Output = Output> + Send + 'static>>>,
);

impl OptionFuture<()> {
    pub fn new<Output>(
        future: Option<impl std::future::Future<Output = Output> + Send + 'static>,
    ) -> OptionFuture<Output> {
        OptionFuture(future.map(|fut| Box::pin(fut) as _))
    }
}

impl<Output> OptionFuture<Output> {
    #[allow(unused)]
    pub fn is_some(&self) -> bool {
        self.0.is_some()
    }

    pub fn is_none(&self) -> bool {
        self.0.is_none()
    }
}

impl<Output> Default for OptionFuture<Output> {
    fn default() -> Self {
        Self(None)
    }
}

impl<Output, F: std::future::Future<Output = Output> + Send + 'static> From<Option<F>>
    for OptionFuture<Output>
{
    fn from(value: Option<F>) -> Self {
        OptionFuture::new(value)
    }
}

impl<Output> std::future::Future for OptionFuture<Output> {
    type Output = Output;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        match self.0.as_mut() {
            Some(fut) => fut.as_mut().poll(cx),
            None => std::task::Poll::Pending,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn option_future_some() {
        let mut fut = tokio_test::task::spawn(OptionFuture::new(Some(async { 42 })));
        assert_eq!(fut.poll(), std::task::Poll::Ready(42));
    }

    #[test]
    fn option_future_none() {
        let mut fut = tokio_test::task::spawn(OptionFuture::<()>::default());
        assert!(fut.poll().is_pending());
    }
}
