use std::future::Future;
use std::pin::Pin;

#[derive(Debug)]
pub struct State {
    pub data: anymap3::Map<dyn std::any::Any + Send + Sync>,
}

impl State {
    pub fn new() -> Self {
        State { data: Default::default() }
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

pub trait IntoResponse<Res> {
    fn into_response(self) -> Res;
}

impl<T: Send> IntoResponse<T> for T {
    fn into_response(self) -> T {
        self
    }
}

pub trait FromRequest<Req, State, Res>: Sized
where
    Req: Send,
    State: Send,
    Res: Send,
{
    fn from_request(bundle: &mut Bundle<Req, State, Res>) -> Option<Self>;
}

#[derive(Debug)]
pub struct StopFlag(pub bool);

impl Default for StopFlag {
    fn default() -> Self {
        Self(false)
    }
}

#[derive(Debug)]
pub struct Bundle<Req, State = crate::handler::State, Res = ()> {
    pub request: Req,
    pub state: State,
    pub response: Res,
    pub stop_flag: StopFlag    
}

impl<Req, State, Res> Bundle<Req, State, Res> {
    pub fn new(request: Req, state: State, response: Res) -> Self {
        Bundle { request, state, response, stop_flag: Default::default() }
    }
}

pub trait Handler<T, Req, State, Res>: Clone + Send + Sync + Sized + 'static
where
    State: Send,
{
    type Future: Future<Output = Option<Res>> + Send;

    fn call(&self, bundle: &mut Bundle<Req, State, Res>) -> Self::Future;
}

impl<F, Fut, Output, Req, State, Res> Handler<((),), Req, State, Res> for F
where
    F: Fn() -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Output> + Send + 'static,
    Output: IntoResponse<Res> + 'static,
    Req: Send + 'static,
    State: Send + 'static,
    Res: Send + 'static,
{
    type Future = Pin<Box<dyn Future<Output = Option<Res>> + Send>>;

    fn call(&self, _bundle: &mut Bundle<Req, State, Res>) -> Self::Future {
        let fut = (self)();
        Box::pin(async move { Some(fut.await.into_response()) })
    }
}

impl<F, Output, Req, State, Res> Handler<Option<()>, Req, State, Res> for F
where
    F: Fn() -> Output + Clone + Send + Sync + 'static,
    Output: IntoResponse<Res> + 'static,
    Req: Send,
    State: Send,
    Res: Send,
{
    type Future = std::future::Ready<Option<Res>>;

    fn call(&self, _bundle: &mut Bundle<Req, State, Res>) -> Self::Future {
        std::future::ready(Some((self)().into_response()))
    }
}

macro_rules! impl_handler {
    ([$($ty:ident),*], $last:ident) => {
        #[allow(non_snake_case, unused_mut)]
        impl<F, Fut, Output, Req, State, Res, $($ty,)* $last> Handler<((), $($ty,)* $last,), Req, State, Res> for F
        where
            F: Fn($($ty,)* $last,) -> Fut + Clone + Send + Sync + 'static,
            Fut: Future<Output = Output> + Send + 'static,
            Output: IntoResponse<Res> + 'static,
            Req: Send + 'static,
            State: Send + 'static,
            Res: Send + 'static,
            $( $ty: FromRequest<Req, State, Res> + Send + 'static, )*
            $last: FromRequest<Req, State, Res> + Send + 'static,
        {
            type Future = Pin<Box<dyn Future<Output = Option<Res>> + Send>>;

            fn call(&self, bundle: &mut Bundle<Req, State, Res>) -> Self::Future {
                $(
                    let $ty = match $ty::from_request(bundle) {
                        Some(value) => value,
                        None => return Box::pin(std::future::ready(None)),
                    };
                )*

                let $last = match $last::from_request(bundle) {
                    Some(value) => value,
                    None => return Box::pin(std::future::ready(None)),
                };

                let handler = self.clone();
                Box::pin(async move {
                    let fut = handler($($ty,)* $last,);
                    Some(fut.await.into_response())
                })
            }
        }

        #[allow(non_snake_case, unused_mut)]
        impl<F, Output, Req, State, Res, $($ty,)* $last> Handler<Option<((), $($ty,)* $last,)>, Req, State, Res> for F
        where
            F: Fn($($ty,)* $last,) -> Output + Clone + Send + Sync + 'static,
            Output: IntoResponse<Res> + 'static,
            Req: Send,
            State: Send,
            Res: Send,
            $( $ty: FromRequest<Req, State, Res> + Send, )*
            $last: FromRequest<Req, State, Res> + Send,
        {
            type Future = std::future::Ready<Option<Res>>;

            fn call(&self, bundle: &mut Bundle<Req, State, Res>) -> Self::Future {
                $(
                    let $ty = match $ty::from_request(bundle) {
                        Some(value) => value,
                        None => return std::future::ready(None),
                    };
                )*

                let $last = match $last::from_request(bundle) {
                    Some(value) => value,
                    None => return std::future::ready(None),
                };

                std::future::ready(Some((self)($($ty,)* $last,).into_response()))
            }
        }
    };
}

impl_handler!([], T1);
impl_handler!([T1], T2);
impl_handler!([T1, T2], T3);
impl_handler!([T1, T2, T3], T4);
impl_handler!([T1, T2, T3, T4], T5);
impl_handler!([T1, T2, T3, T4, T5], T6);
impl_handler!([T1, T2, T3, T4, T5, T6], T7);
impl_handler!([T1, T2, T3, T4, T5, T6, T7], T8);
impl_handler!([T1, T2, T3, T4, T5, T6, T7, T8], T9);
impl_handler!([T1, T2, T3, T4, T5, T6, T7, T8, T9], T10);
impl_handler!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10], T11);
impl_handler!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11], T12);
impl_handler!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12], T13);
impl_handler!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13], T14);
impl_handler!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14], T15);
impl_handler!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15], T16);

pub trait Call<Req, State, Res>: Send + Sync {
    fn call(&self, bundle: Bundle<Req, State, Res>) -> Pin<Box<dyn Future<Output = Bundle<Req, State, Res>> + Send>>;
    fn priority(&self) -> u8;
}

pub mod tower_handler {
    use std::convert::Infallible;
    use std::future::Future;
    use std::marker::PhantomData;
    use std::pin::Pin;
    use std::task::Poll;

    use tower::Service;
    use tower::ServiceExt;
    use tower::util::BoxCloneService;

    use super::Bundle;
    use super::Call;
    use super::Handler;

    struct HandlerService<H, T, Req, State, Res> {
        handler: H,
        _marker: PhantomData<fn() -> (T, Req, State, Res)>,
    }

    impl<H, T, Req, State, Res> HandlerService<H, T, Req, State, Res> {
        fn new(handler: H) -> Self {
            Self { handler, _marker: PhantomData }
        }
    }

    impl<H, T, Req, State, Res> Clone for HandlerService<H, T, Req, State, Res>
    where
        H: Clone,
    {
        fn clone(&self) -> Self {
            Self { handler: self.handler.clone(), _marker: PhantomData }
        }
    }

    impl<H, T, Req, State, Res> Service<Bundle<Req, State, Res>> for HandlerService<H, T, Req, State, Res>
    where
        H: Handler<T, Req, State, Res> + Clone,
        Req: Send + 'static,
        State: Send + 'static,
        Res: Send + 'static + std::ops::Mul<Output = Res>,
    {
        type Response = Bundle<Req, State, Res>;
        type Error = Infallible;
        type Future = HandlerServiceFuture<H::Future, Req, State, Res>;

        fn call(&mut self, mut bundle: Bundle<Req, State, Res>) -> Self::Future {
            let future = self.handler.call(&mut bundle);
            HandlerServiceFuture { future, bundle: Some(bundle) }
        }

        fn poll_ready(&mut self, _: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    pin_project_lite::pin_project! {
                struct HandlerServiceFuture<F, Req, State, Res> {
            #[pin]
            future: F,
            bundle: Option<Bundle<Req, State, Res>>,
        }
    }

    impl<F, Req, State, Res> Future for HandlerServiceFuture<F, Req, State, Res>
    where
        F: Future<Output = Option<Res>>,
        Res: std::ops::Mul<Output = Res>,
    {
        type Output = Result<Bundle<Req, State, Res>, Infallible>;

        fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
            let this = self.project();
            match this.future.poll(cx) {
                Poll::Ready(Some(response)) => {
                    let mut bundle = this.bundle.take().unwrap();
                    bundle.response = bundle.response * response;
                    Poll::Ready(Ok(bundle))
                }
                Poll::Ready(None) => {
                    Poll::Ready(Ok(this.bundle.take().unwrap()))
                }
                Poll::Pending => Poll::Pending,
            }
        }
    }

    pub struct TowerHandler<Req, State, Res> {
        pub service: BoxCloneService<Bundle<Req, State, Res>, Bundle<Req, State, Res>, Infallible>,
        pub priority: u8,
        pub name: &'static str,
        pub module_name: &'static str,
    }

    unsafe impl<Req, State, Res> Sync for TowerHandler<Req, State, Res> {}

    impl<Req, State, Res> TowerHandler<Req, State, Res>
    where
        Req: Send + 'static,
        State: Send + 'static,
        Res: Send + std::ops::Mul<Output = Res> + 'static,
    {
        pub fn new<T: 'static>(
            priority: u8,
            name: &'static str,
            module_name: &'static str,
            handler: impl Handler<T, Req, State, Res>,
        ) -> Self {
            let service = HandlerService::new(handler);
            Self {
                priority,
                name,
                module_name,
                service: BoxCloneService::new(service),
            }
        }
    }

    impl<Req, State, Res> Service<Bundle<Req, State, Res>> for TowerHandler<Req, State, Res>
    where
        Req: Send + 'static,
        State: Send + 'static,
        Res: Send + 'static,
    {
        type Response = Bundle<Req, State, Res>;
        type Error = Infallible;
        type Future = futures::future::BoxFuture<'static, Result<Bundle<Req, State, Res>, Infallible>>;

        fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.service.poll_ready(cx)
        }

        fn call(&mut self, bundle: Bundle<Req, State, Res>) -> Self::Future {
            self.service.call(bundle)
        }
    }

    impl<Req, State, Res> Clone for TowerHandler<Req, State, Res> {
        fn clone(&self) -> Self {
            Self {
                service: self.service.clone(),
                priority: self.priority,
                name: self.name,
                module_name: self.module_name,
            }
        }
    }

    impl<Req, State, Res> Call<Req, State, Res> for TowerHandler<Req, State, Res>
    where
        Req: Send + 'static,
        State: Send + 'static,
        Res: Send + 'static,
    {
        fn call(&self, bundle: Bundle<Req, State, Res>) -> Pin<Box<dyn Future<Output = Bundle<Req, State, Res>> + Send>> {
            let service = self.service.clone();
            Box::pin(async move { service.oneshot(bundle).await.unwrap() })
        }

        fn priority(&self) -> u8 {
            self.priority
        }
    }
}

pub mod async_handler {
    use std::future::Future;
    use std::marker::PhantomData;
    use std::pin::Pin;
    use std::sync::Arc;

    use super::Bundle;
    use super::Call;
    use super::Handler;

    struct HandlerWrapper<H, T, Req, State, Res> {
        handler: H,
        priority: u8,
        _phantom: PhantomData<fn() -> (T, Req, State, Res)>,
    }

    impl<H, T, Req, State, Res> Clone for HandlerWrapper<H, T, Req, State, Res>
    where
        H: Clone,
    {
        fn clone(&self) -> Self {
            Self {
                handler: self.handler.clone(),
                priority: self.priority,
                _phantom: PhantomData,
            }
        }
    }

    impl<H, T, Req, State, Res> Call<Req, State, Res> for HandlerWrapper<H, T, Req, State, Res>
    where
        H: Handler<T, Req, State, Res>,
        <H as Handler<T, Req, State, Res>>::Future: 'static,
        T: 'static,
        Req: Send + 'static,
        State: Send + 'static,
        Res: Send + std::ops::Mul<Output = Res> + 'static,
    {
        fn call(&self, bundle: Bundle<Req, State, Res>) -> Pin<Box<dyn Future<Output = Bundle<Req, State, Res>> + Send>> {
            let handler = self.handler.clone();
            Box::pin(async move {
                let mut bundle = bundle;
                if let Some(response) = handler.call(&mut bundle).await {
                    bundle.response = bundle.response * response;
                }
                bundle
            })
        }

        fn priority(&self) -> u8 {
            self.priority
        }
    }

    pub struct AsyncHandler<Req, State, Res> {
        pub name: &'static str,
        pub module_name: &'static str,
        handler: Arc<dyn Call<Req, State, Res>>,
    }

    impl<Req, State, Res> Clone for AsyncHandler<Req, State, Res> {
        fn clone(&self) -> Self {
            Self {
                name: self.name,
                module_name: self.module_name,
                handler: self.handler.clone(),
            }
        }
    }

    impl<Req, State, Res> AsyncHandler<Req, State, Res>
    where
        Req: Send + 'static,
        State: Send + 'static,
        Res: Send + std::ops::Mul<Output = Res> + 'static,
    {
        pub fn new<T: 'static, H: Handler<T, Req, State, Res>>(
            priority: u8,
            name: &'static str,
            module_name: &'static str,
            handler: H,
        ) -> Self
        where
            <H as Handler<T, Req, State, Res>>::Future: 'static,
        {
            let wrapper = HandlerWrapper {
                handler,
                priority,
                _phantom: PhantomData,
            };
            Self {
                name,
                module_name,
                handler: Arc::new(wrapper),
            }
        }
    }

    impl<Req, State, Res> Call<Req, State, Res> for AsyncHandler<Req, State, Res>
    where
        Req: Send + 'static,
        State: Send + 'static,
        Res: Send + 'static,
    {
        fn call(&self, bundle: Bundle<Req, State, Res>) -> Pin<Box<dyn Future<Output = Bundle<Req, State, Res>> + Send>> {
            self.handler.call(bundle)
        }

        fn priority(&self) -> u8 {
            self.handler.priority()
        }
    }
}

pub mod sync_handler {
    use std::future::Future;
    use std::pin::Pin;

    use super::Bundle;
    use super::Call;
    use super::Handler;

    /// Type-erased synchronous handler wrapper.
    ///
    /// Uses raw pointers and monomorphized function pointers to achieve type erasure
    /// without requiring `'static` bounds on `Req`, `State`, `Res`, or the handler's
    /// type parameter `T`. The caller is responsible for ensuring soundness.
    pub struct SyncHandler<Req, State, Res> {
        pub name: &'static str,
        pub module_name: &'static str,
        priority: u8,
        handler_pointer: *const (),
        call_fn: unsafe fn(*const (), &mut Bundle<Req, State, Res>) -> Option<Res>,
        clone_fn: unsafe fn(*const ()) -> *const (),
        drop_fn: unsafe fn(*const ()),
    }

    /// Safety: the stored handler satisfies `Send + Sync` via the `Handler` trait bound.
    unsafe impl<Req, State, Res> Send for SyncHandler<Req, State, Res> {}
    /// Safety: the stored handler satisfies `Send + Sync` via the `Handler` trait bound.
    unsafe impl<Req, State, Res> Sync for SyncHandler<Req, State, Res> {}

    impl<Req, State, Res> SyncHandler<Req, State, Res>
    where
        Req: Send,
        State: Send,
        Res: Send,
    {
        pub fn new<T, H: Handler<T, Req, State, Res, Future = std::future::Ready<Option<Res>>>>(
            priority: u8,
            name: &'static str,
            module_name: &'static str,
            handler: H,
        ) -> Self {
            let handler_pointer = Box::into_raw(Box::new(handler)) as *const ();

            unsafe fn call_erased<H, T, Req, State, Res>(
                pointer: *const (),
                bundle: &mut Bundle<Req, State, Res>,
            ) -> Option<Res>
            where
                H: Handler<T, Req, State, Res, Future = std::future::Ready<Option<Res>>>,
                State: Send,
            {
                let handler = unsafe { &*(pointer as *const H) };
                handler.call(bundle).into_inner()
            }

            unsafe fn clone_erased<H: Clone>(pointer: *const ()) -> *const () {
                let handler = unsafe { &*(pointer as *const H) };
                Box::into_raw(Box::new(handler.clone())) as *const ()
            }

            unsafe fn drop_erased<H>(pointer: *const ()) {
                drop(unsafe { Box::from_raw(pointer as *mut H) });
            }

            Self {
                name,
                module_name,
                priority,
                handler_pointer,
                call_fn: call_erased::<H, T, Req, State, Res>,
                clone_fn: clone_erased::<H>,
                drop_fn: drop_erased::<H>,
            }
        }
    }

    impl<Req, State, Res> Clone for SyncHandler<Req, State, Res> {
        fn clone(&self) -> Self {
            Self {
                name: self.name,
                module_name: self.module_name,
                priority: self.priority,
                handler_pointer: unsafe { (self.clone_fn)(self.handler_pointer) },
                call_fn: self.call_fn,
                clone_fn: self.clone_fn,
                drop_fn: self.drop_fn,
            }
        }
    }

    impl<Req, State, Res> Drop for SyncHandler<Req, State, Res> {
        fn drop(&mut self) {
            unsafe { (self.drop_fn)(self.handler_pointer) }
        }
    }

    impl<Req, State, Res> Call<Req, State, Res> for SyncHandler<Req, State, Res>
    where
        Req: Send,
        State: Send,
        Res: Send + std::ops::Mul<Output = Res>,
    {
        fn call(&self, bundle: Bundle<Req, State, Res>) -> Pin<Box<dyn Future<Output = Bundle<Req, State, Res>> + Send>> {
            let mut bundle = bundle;
            let result = unsafe { (self.call_fn)(self.handler_pointer, &mut bundle) };
            if let Some(response) = result {
                bundle.response = bundle.response * response;
            }
            let boxed: Box<dyn Future<Output = Bundle<Req, State, Res>> + Send> = Box::new(std::future::ready(bundle));
            unsafe {
                let raw = Box::into_raw(boxed);
                Pin::from(Box::from_raw(std::mem::transmute::<
                    *mut (dyn Future<Output = Bundle<Req, State, Res>> + Send),
                    *mut (dyn Future<Output = Bundle<Req, State, Res>> + Send + 'static),
                >(raw)))
            }
        }

        fn priority(&self) -> u8 {
            self.priority
        }
    }
}
