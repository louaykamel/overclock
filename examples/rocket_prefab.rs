#[cfg(feature = "rocket")]
use overclock::prefab::rocket::*;
#[cfg(feature = "rocket")]
use rocket::get;

#[cfg(feature = "rocket")]
async fn construct_rocket() -> Result<::rocket::Rocket<rocket::Ignite>, rocket::Error> {
    rocket::build()
        .mount("/", rocket::routes![info])
        .attach(CORS)
        .attach(RequestTimer::default())
        .register("/", rocket::catchers![internal_error, not_found])
        .ignite()
        .await
}

#[get("/info")]
async fn info() -> &'static str {
    "Got info endpoint!"
}

#[cfg(feature = "rocket")]
// our prefab example starts from here
use overclock::core::Runtime;
#[tokio::main]
async fn main() {
    #[cfg(not(feature = "console"))]
    {
        let env = env_logger::Env::new().filter_or("RUST_LOG", "info");
        env_logger::Builder::from_env(env).init();
    }
    let user_defined_rocket = construct_rocket()
        .await
        .expect("user defined rocket to be constructed");
    let rocket = RocketServer::new(user_defined_rocket);
    let runtime = Runtime::new("rocket".to_string(), rocket)
        .await
        .expect("Runtime to run");
    runtime
        .block_on()
        .await
        .expect("runtime to gracefuly shutdown")
}
