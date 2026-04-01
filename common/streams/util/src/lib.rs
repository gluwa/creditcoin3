#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct RootInfo {
    pub height: attestor_primitives::Height,
    pub root: attestor_primitives::Digest,
    pub hash: attestor_primitives::Digest,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttestationInfo {
    pub height: attestor_primitives::Height,
    pub digest: attestor_primitives::Digest,
}

/// [`ChainData`] is not dyn-compatible but is easier to implement and more versatile to use.
pub trait ChainData<T>: futures::Stream<Item = T> + Unpin {
    fn reset(&self, info: AttestationInfo) -> impl std::future::Future<Output = Self> + Send;
}

pub trait ChainExt<T>
where
    Self: ChainData<T> + sealed::Sealed<T> + Send + Sync + Sized + 'static,
    T: 'static,
{
    fn boxed_data(self) -> BoxedData<T> {
        Box::pin(self)
    }
}

impl<T, C> ChainExt<T> for C
where
    C: ChainData<T> + Send + Sync + Sized + 'static,
    T: 'static,
{
}

pub type BoxedData<T> = std::pin::Pin<Box<dyn sealed::DynData<T> + Send + Sync>>;
pub type BoxedStream<T> = std::pin::Pin<Box<dyn futures::Stream<Item = T> + Send>>;

mod sealed {
    use super::*;

    pub trait Sealed<T> {}
    impl<T, C: ChainData<T>> Sealed<T> for C {}

    /// [`DynData`] **is** a dyn-compatible wrapper which makes it possible to use types which
    /// implement [`ChainData`] in a dyn-context.
    pub trait DynData<T>: futures::Stream<Item = T> + Unpin + Sealed<T> {
        fn reset_boxed(
            &self,
            info: AttestationInfo,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = BoxedData<T>> + Send + '_>>;
    }

    /// Glue which converts non-dyn-compatible [`ChainData`] types into dyn-compatible [`DynData`]
    /// implementors.
    impl<C, T> DynData<T> for C
    where
        C: ChainData<T> + Send + Sync + 'static,
        T: 'static,
    {
        fn reset_boxed(
            &self,
            info: AttestationInfo,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = BoxedData<T>> + Send + '_>>
        {
            use futures::FutureExt as _;

            self.reset(info)
                .map(|s| Box::pin(s) as BoxedData<T>)
                .boxed()
        }
    }

    impl<T: 'static> ChainData<T> for std::pin::Pin<Box<dyn sealed::DynData<T> + Send + Sync>> {
        /// Make sure to call [`reset_boxed`] on the inner [`DynData`] to avoid recurring on this
        /// call site.
        ///
        /// [`reset_boxed`]: DynData::reset_boxed
        async fn reset(&self, info: AttestationInfo) -> Self {
            (**self).reset_boxed(info).await
        }
    }
}
