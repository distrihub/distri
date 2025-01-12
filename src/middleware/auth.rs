use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::{Error, HttpMessage};
use firebase_auth::FirebaseAuth;
use futures::future::{ready, LocalBoxFuture, Ready};
use std::rc::Rc;

pub struct FirebaseAuthMiddleware {
    auth: Rc<FirebaseAuth>,
}

impl FirebaseAuthMiddleware {
    pub fn new(auth: FirebaseAuth) -> Self {
        Self {
            auth: Rc::new(auth),
        }
    }
}

impl<S> Transform<S, ServiceRequest> for FirebaseAuthMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse, Error = Error> + 'static,
{
    type Response = ServiceResponse;
    type Error = Error;
    type Transform = FirebaseAuthMiddlewareService<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(FirebaseAuthMiddlewareService {
            service,
            auth: self.auth.clone(),
        }))
    }
}

pub struct FirebaseAuthMiddlewareService<S> {
    service: S,
    auth: Rc<FirebaseAuth>,
}

impl<S> Service<ServiceRequest> for FirebaseAuthMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse, Error = Error> + 'static,
{
    type Response = ServiceResponse;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn call(&self, req: ServiceRequest) -> Self::Future {
        // ... implement token verification logic
    }
} 