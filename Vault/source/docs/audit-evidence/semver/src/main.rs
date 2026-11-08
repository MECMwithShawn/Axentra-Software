use std::{env,fs};
use serde_json::Value;
fn main(){
 let input=fs::read_to_string(env::args().nth(1).unwrap()).unwrap();
 let candidates:Vec<Value>=serde_json::from_str(&input).unwrap();
 let found:Vec<_>=candidates.into_iter().filter(|c|{
   let v=semver::Version::parse(c["version"].as_str().unwrap()).unwrap();
   !["patched","unaffected"].iter().any(|field|
     c["versions"][field].as_array().is_some_and(|a|a.iter().any(|req|
       semver::VersionReq::parse(req.as_str().unwrap()).unwrap().matches(&v))))
 }).collect();
 println!("{}",serde_json::to_string_pretty(&found).unwrap());
}
