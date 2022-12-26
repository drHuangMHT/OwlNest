use tonic::{Request, Response, Status};

use hello_world::greeter_server::Greeter;
use hello_world::*;
use tracing::info;

pub mod hello_world {
    tonic::include_proto!("helloworld");
}

#[derive(Debug, Default)]
pub struct GreeterRequestHandler {}

#[tonic::async_trait]
impl Greeter for GreeterRequestHandler {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>, // Accept request of type HelloRequest
    ) -> Result<Response<HelloReply>, Status> {
        // Return an instance of type HelloReply
        println!("Got a request: {:?}", request);

        let reply = hello_world::HelloReply {
            message: format!("Hello {}!", request.into_inner().name).into(), // We must use .into_inner() as the fields of gRPC requests and responses are private
        };

        Ok(Response::new(reply)) // Send back our formatted greeting
    }
    async fn hb(&self, request: Request<HbRequest>) -> Result<Response<HbReply>, Status> {
        info!("Got Hb request: {:?}",request);
        let reply = hello_world::HbReply{
            rand: rand::random::<i64>()
        };
        Ok(Response::new(reply))
    }
}
