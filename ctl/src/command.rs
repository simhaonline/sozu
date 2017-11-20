use sozu_command::config::Config;
use sozu_command::channel::Channel;
use sozu_command::certificate::{calculate_fingerprint,split_certificate_chain};
use sozu_command::data::{AnswerData,ConfigCommand,ConfigMessage,ConfigMessageAnswer,ConfigMessageStatus,RunState};
use sozu_command::messages::{Application, Order, Instance, HttpFront, HttpsFront, CertificateAndKey, CertFingerprint, TcpFront, Query};

use std::collections::{HashMap,HashSet};
use std::process::exit;
use std::thread;
use std::sync::{Arc,Mutex};
use std::time::Duration;
use std::sync::mpsc;
use rand::{thread_rng, Rng};
use prettytable::Table;
use prettytable::row::Row;
use prettytable::cell::Cell;

fn generate_id() -> String {
  let s: String = thread_rng().gen_ascii_chars().take(6).collect();
  format!("ID-{}", s)
}

fn generate_tagged_id(tag: &str) -> String {
  let s: String = thread_rng().gen_ascii_chars().take(6).collect();
  format!("{}-{}", tag, s)
}


pub fn save_state(channel: &mut Channel<ConfigMessage,ConfigMessageAnswer>, path: &str) {
  let id = generate_id();
  channel.write_message(&ConfigMessage::new(
    id.clone(),
    ConfigCommand::SaveState(path.to_string()),
    None,
  ));

  match channel.read_message() {
    None          => {
      println!("the proxy didn't answer");
      exit(1);
    },
    Some(message) => {
      if id != message.id {
        println!("received message with invalid id: {:?}", message);
        exit(1);
      }
      match message.status {
        ConfigMessageStatus::Processing => {
          // do nothing here
          // for other messages, we would loop over read_message
          // until an error or ok message was sent
        },
        ConfigMessageStatus::Error => {
          println!("could not save proxy state: {}", message.message);
          exit(1);
        },
        ConfigMessageStatus::Ok => {
          println!("{}", message.message);
        }
      }
    }
  }
}

pub fn load_state(channel: &mut Channel<ConfigMessage,ConfigMessageAnswer>, path: &str) {
  let id = generate_id();
  channel.write_message(&ConfigMessage::new(
    id.clone(),
    ConfigCommand::LoadState(path.to_string()),
    None,
  ));

  match channel.read_message() {
    None          => {
      println!("the proxy didn't answer");
      exit(1);
    },
    Some(message) => {
      if id != message.id {
        println!("received message with invalid id: {:?}", message);
        exit(1);
      }
      match message.status {
        ConfigMessageStatus::Processing => {
          // do nothing here
          // for other messages, we would loop over read_message
          // until an error or ok message was sent
        },
        ConfigMessageStatus::Error => {
          println!("could not load proxy state: {}", message.message);
          exit(1);
        },
        ConfigMessageStatus::Ok => {
          println!("Proxy state loaded successfully from {}", path);
        }
      }
    }
  }
}

pub fn dump_state(channel: &mut Channel<ConfigMessage,ConfigMessageAnswer>) {
  let id = generate_id();
  channel.write_message(&ConfigMessage::new(
    id.clone(),
    ConfigCommand::DumpState,
    None,
  ));

  match channel.read_message() {
    None          => {
      println!("the proxy didn't answer");
      exit(1);
    },
    Some(message) => {
      if id != message.id {
        println!("received message with invalid id: {:?}", message);
        exit(1);
      }
      match message.status {
        ConfigMessageStatus::Processing => {
          // do nothing here
          // for other messages, we would loop over read_message
          // until an error or ok message was sent
        },
        ConfigMessageStatus::Error => {
          println!("could not dump proxy state: {}", message.message);
          exit(1);
        },
        ConfigMessageStatus::Ok => {
          if let Some(AnswerData::State(state)) = message.data {
            println!("{:#?}", state);
          } else {
            println!("state dump was empty");
            exit(1);
          }
        }
      }
    }
  }
}

pub fn soft_stop(channel: &mut Channel<ConfigMessage,ConfigMessageAnswer>) {
  println!("shutting down proxy");
  let id = generate_id();
  channel.write_message(&ConfigMessage::new(
    id.clone(),
    ConfigCommand::ProxyConfiguration(Order::SoftStop),
    //FIXME: we should be able to soft stop one specific worker
    None,
  ));

  loop {
    match channel.read_message() {
      None          => println!("the proxy didn't answer"),
      Some(message) => {
        if &id != &message.id {
          println!("received message with invalid id: {:?}", message);
          return;
        }
        match message.status {
          ConfigMessageStatus::Processing => {
            println!("Proxy is processing: {}", message.message);
          },
          ConfigMessageStatus::Error => {
            println!("could not stop the proxy: {}", message.message);
          },
          ConfigMessageStatus::Ok => {
            println!("Proxy shut down: {}", message.message);
            break;
          }
        }
      }
    }
  }
}

pub fn hard_stop(channel: &mut Channel<ConfigMessage,ConfigMessageAnswer>) {
  println!("shutting down proxy");
  let id = generate_id();
  channel.write_message(&ConfigMessage::new(
    id.clone(),
    ConfigCommand::ProxyConfiguration(Order::HardStop),
    //FIXME: we should be able to soft stop one specific worker
    None,
  ));

  loop {
    match channel.read_message() {
      None          => println!("the proxy didn't answer"),
      Some(message) => {
        match message.status {
          ConfigMessageStatus::Processing => {
            println!("Proxy is processing: {}", message.message);
          },
          ConfigMessageStatus::Error => {
            println!("could not stop the proxy: {}", message.message);
          },
          ConfigMessageStatus::Ok => {
            if &id == &message.id {
              println!("Proxy shut down: {}", message.message);
              break;
            }
          }
        }
      }
    }
  }
}

pub fn upgrade(channel: &mut Channel<ConfigMessage,ConfigMessageAnswer>) {
  let id = generate_tagged_id("LIST-WORKERS");
  channel.write_message(&ConfigMessage::new(
    id.clone(),
    ConfigCommand::ListWorkers,
    None,
  ));

  match channel.read_message() {
    None          => println!("the proxy didn't answer"),
    Some(message) => {
      if id != message.id {
        println!("received message with invalid id: {:?}", message);
        return;
      }
      match message.status {
        ConfigMessageStatus::Processing => {
          println!("should have obtained an answer immediately");
          return;
        },
        ConfigMessageStatus::Error => {
          println!("could not get the worker list: {}", message.message);
          return
        },
        ConfigMessageStatus::Ok => {
          println!("Worker list:\n{:?}", message.data);
          if let Some(AnswerData::Workers(ref workers)) = message.data {
            let mut launching: HashSet<String> = HashSet::new();
            let mut stopping:  HashSet<String> = HashSet::new();

            for ref worker in workers.iter().filter(|worker| worker.run_state == RunState::Running) {
              let id = generate_tagged_id("LAUNCH-WORKER");
              let msg = ConfigMessage::new(
                id.clone(),
                ConfigCommand::LaunchWorker("BLAH".to_string()),
                None,
              );
              println!("sending message: {:?}", msg);
              channel.write_message(&msg);
              launching.insert(id);
            }

            for ref worker in workers.iter().filter(|worker| worker.run_state == RunState::Running) {
              let id = generate_tagged_id("SOFT-STOP-WORKER");
              let msg = ConfigMessage::new(
                id.clone(),
                ConfigCommand::ProxyConfiguration(Order::SoftStop),
                Some(worker.id),
              );
              println!("sending message: {:?}", msg);
              channel.write_message(&msg);
              stopping.insert(id);
            }


            loop {
              println!("launching: {:?}\nstopping: {:?}", launching, stopping);
              if launching.is_empty() && stopping.is_empty() {
                break;
              }
              match channel.read_message() {
                None          => println!("the proxy didn't answer"),
                Some(message) => {
                  println!("received message: {:?}", message);
                  match message.status {
                    ConfigMessageStatus::Processing => {
                    },
                    ConfigMessageStatus::Error => {
                      println!("error for message[{}]: {}", message.id, message.message);
                      if launching.contains(&message.id) {
                        launching.remove(&message.id);
                        println!("launch message with ID {} done", message.id);
                      }
                      if stopping.contains(&message.id) {
                        stopping.remove(&message.id);
                        println!("stop message with ID {} done", message.id);
                      }
                    },
                    ConfigMessageStatus::Ok => {
                      if launching.contains(&message.id) {
                        launching.remove(&message.id);
                        println!("launch message with ID {} done", message.id);
                      }
                      if stopping.contains(&message.id) {
                        stopping.remove(&message.id);
                        println!("stop message with ID {} done", message.id);
                      }
                    }
                  }
                }
              }
            }

            println!("worker upgrade done");
            let id = generate_tagged_id("UPGRADE-MASTER");
            channel.write_message(&ConfigMessage::new(
              id.clone(),
              ConfigCommand::UpgradeMaster,
              None,
            ));
            println!("master upgrade message sent: {}", id);

            loop {
              match channel.read_message() {
                None          => println!("the proxy didn't answer"),
                Some(message) => {
                  if &id != &message.id {
                    println!("received message with invalid id: {:?}", message);
                    return;
                  }
                  match message.status {
                    ConfigMessageStatus::Processing => {
                      println!("master is processing: {}", message.message);
                    },
                    ConfigMessageStatus::Error => {
                      println!("could not upgrade the master: {}", message.message);
                      return;
                    },
                    ConfigMessageStatus::Ok => {
                      println!("successfully upgraded the master: {}", message.message);
                      return;
                    }
                  }
                }
              }
            }
          }
        }
      }
    }
  }
}

pub fn status(mut channel: Channel<ConfigMessage,ConfigMessageAnswer>) {
  let id = generate_id();
  channel.write_message(&ConfigMessage::new(
    id.clone(),
    ConfigCommand::ListWorkers,
    None,
  ));

  match channel.read_message() {
    None          => {
      println!("the proxy didn't answer");
      exit(1);
    },
    Some(message) => {
      if id != message.id {
        println!("received message with invalid id: {:?}", message);
        exit(1);
      }
      match message.status {
        ConfigMessageStatus::Processing => {
          println!("should have obtained an answer immediately");
          exit(1);
        },
        ConfigMessageStatus::Error => {
          println!("could not get the worker list: {}", message.message);
          exit(1);
        },
        ConfigMessageStatus::Ok => {
          //println!("Worker list:\n{:?}", message.data);
          if let Some(AnswerData::Workers(ref workers)) = message.data {
            let mut expecting: HashSet<String> = HashSet::new();

            let mut h = HashMap::new();
            for ref worker in workers.iter().filter(|worker| worker.run_state == RunState::Running) {
              let id = generate_id();
              let msg = ConfigMessage::new(
                id.clone(),
                ConfigCommand::ProxyConfiguration(Order::Status),
                Some(worker.id),
              );
              //println!("sending message: {:?}", msg);
              channel.write_message(&msg);
              expecting.insert(id.clone());
              h.insert(id, (worker.id, ConfigMessageStatus::Processing));
            }

            let state = Arc::new(Mutex::new(h));
            let st = state.clone();
            let (send, recv) = mpsc::channel();

            thread::spawn(move || {
              loop {
                //println!("expecting: {:?}", expecting);
                if expecting.is_empty() {
                  break;
                }
                match channel.read_message() {
                  None          => {
                    println!("the proxy didn't answer");
                    exit(1);
                  },
                  Some(message) => {
                    //println!("received message: {:?}", message);
                    match message.status {
                      ConfigMessageStatus::Processing => {
                      },
                      ConfigMessageStatus::Error => {
                        println!("error for message[{}]: {}", message.id, message.message);
                        if expecting.contains(&message.id) {
                          expecting.remove(&message.id);
                          //println!("status message with ID {} done", message.id);
                          if let Ok(mut h) = state.try_lock() {
                            if let Some(data) = h.get_mut(&message.id) {
                              *data = ((*data).0, ConfigMessageStatus::Error);
                            }
                          }
                        }
                      },
                      ConfigMessageStatus::Ok => {
                        if expecting.contains(&message.id) {
                          expecting.remove(&message.id);
                          //println!("status message with ID {} done", message.id);
                          if let Ok(mut h) = state.try_lock() {
                            if let Some(data) = h.get_mut(&message.id) {
                              *data = ((*data).0, ConfigMessageStatus::Ok);
                            }
                          }
                        }
                      }
                    }
                  }
                }
              }

              send.send(()).unwrap();
            });

            let finished = recv.recv_timeout(Duration::from_millis(1000)).is_ok();
            let placeholder = if finished {
              String::from("")
            } else {
              String::from("timeout")
            };

            let mut h2: HashMap<u32, String> = if let Ok(mut state) = st.try_lock() {
              state.values().map(|&(ref id, ref status)| {
                (*id, String::from(match *status {
                  ConfigMessageStatus::Processing => if finished {
                    "processing"
                  } else {
                    "timeout"
                  },
                  ConfigMessageStatus::Error      => "error",
                  ConfigMessageStatus::Ok         => "ok",
                }))
              }).collect()
            } else {
              HashMap::new()
            };

            let mut table = Table::new();

            let empty = "";
            table.add_row(row!["Worker", "pid", "run state", "answer"]);
            for ref worker in workers.iter() {
              let run_state = format!("{:?}", worker.run_state);
              table.add_row(row![worker.id, worker.pid, run_state, h2.get(&worker.id).unwrap_or(&placeholder)]);
            }

            table.printstd();

          }
        }
      }
    }
  }
}

pub fn metrics(channel: &mut Channel<ConfigMessage,ConfigMessageAnswer>) {
  let id = generate_id();
  //println!("will send message for metrics with id {}", id);
  channel.write_message(&ConfigMessage::new(
    id.clone(),
    ConfigCommand::Metrics,
    None,
  ));
  //println!("message sent");

  loop {
    match channel.read_message() {
      None          => println!("the proxy didn't answer"),
      Some(message) => {
        match message.status {
          ConfigMessageStatus::Processing => {
            println!("Proxy is processing: {}", message.message);
          },
          ConfigMessageStatus::Error => {
            println!("could not stop the proxy: {}", message.message);
          },
          ConfigMessageStatus::Ok => {
            if &id == &message.id {
              //println!("Sozu metrics:\n{}\n{:#?}", message.message, message.data);

              if let Some(AnswerData::Metrics(mut data)) = message.data {
                if let Some(master) = data.remove("master") {
                  let mut master_table = Table::new();
                  master_table.add_row(row![String::from("key"), String::from("value")]);

                  println!("master process metrics:\n");
                  for (ref key, ref value) in master.proxy.iter() {
                    master_table.add_row(row![key.to_string(), format!("{:?}", value)]);
                     //println!("{}:\t{:?}", key, value);
                  }

                  master_table.printstd();
                }

                println!("\nworker metrics:\n");

                let mut proxy_table = Table::new();
                let mut header = Vec::new();
                header.push(cell!("key"));
                for ref key in data.keys() {
                  header.push(cell!(&key));
                }
                proxy_table.add_row(Row::new(header));

                let mut application_table = Table::new();
                let mut header = Vec::new();
                header.push(cell!("application"));
                for ref key in data.keys() {
                  header.push(cell!(&format!("{} samples", key)));
                  header.push(cell!(&format!("{} p50%", key)));
                  header.push(cell!(&format!("{} p90%", key)));
                  header.push(cell!(&format!("{} p99%", key)));
                  header.push(cell!(&format!("{} p99.9%", key)));
                  header.push(cell!(&format!("{} p99.99%", key)));
                  header.push(cell!(&format!("{} p99.999%", key)));
                  header.push(cell!(&format!("{} p100%", key)));
                }
                application_table.add_row(Row::new(header));

                let mut backend_table = Table::new();
                let mut header = Vec::new();
                header.push(cell!("backend"));
                for ref key in data.keys() {
                  header.push(cell!(&format!("{} bytes out", key)));
                  header.push(cell!(&format!("{} bytes in", key)));
                  header.push(cell!(&format!("{} samples", key)));
                  header.push(cell!(&format!("{} p50%", key)));
                  header.push(cell!(&format!("{} p90%", key)));
                  header.push(cell!(&format!("{} p99%", key)));
                  header.push(cell!(&format!("{} p99.9%", key)));
                  header.push(cell!(&format!("{} p99.99%", key)));
                  header.push(cell!(&format!("{} p99.999%", key)));
                  header.push(cell!(&format!("{} p100%", key)));
                }
                backend_table.add_row(Row::new(header));


                let mut proxy_data = HashMap::new();
                let mut application_data = HashMap::new();
                let mut backend_data = HashMap::new();

                for ref metrics in data.values() {
                  for (ref key, ref value) in metrics.proxy.iter() {
                    (*(proxy_data.entry(key.clone()).or_insert(Vec::new()))).push(value.clone());
                  }

                  for (ref key, ref percentiles) in metrics.applications.iter() {
                    let mut entry = application_data.entry(key.clone()).or_insert(Vec::new());
                    (*entry).push(percentiles.samples);
                    (*entry).push(percentiles.p_50);
                    (*entry).push(percentiles.p_90);
                    (*entry).push(percentiles.p_99);
                    (*entry).push(percentiles.p_99_9);
                    (*entry).push(percentiles.p_99_99);
                    (*entry).push(percentiles.p_99_999);
                    (*entry).push(percentiles.p_100);
                  }

                  for (ref key, ref back) in metrics.backends.iter() {
                    let mut entry = backend_data.entry(key.clone()).or_insert(Vec::new());
                    let bout = back.bytes_out as u64;
                    let bin  = back.bytes_in as u64;
                    (*entry).push(bout);
                    (*entry).push(bin);
                    (*entry).push(back.percentiles.samples);
                    (*entry).push(back.percentiles.p_50);
                    (*entry).push(back.percentiles.p_90);
                    (*entry).push(back.percentiles.p_99);
                    (*entry).push(back.percentiles.p_99_9);
                    (*entry).push(back.percentiles.p_99_99);
                    (*entry).push(back.percentiles.p_99_999);
                    (*entry).push(back.percentiles.p_100);
                  }
                }

                for (ref key, ref values) in proxy_data.iter() {
                  let mut row = Vec::new();
                  row.push(cell!(key));

                  for val in values.iter() {
                    row.push(cell!(format!("{:?}", val)));
                  }

                  proxy_table.add_row(Row::new(row));
                }

                proxy_table.printstd();

                println!("\napplication metrics:\n");

                for (ref key, ref values) in application_data.iter() {
                  let mut row = Vec::new();
                  row.push(cell!(key));

                  for val in values.iter() {
                    row.push(cell!(format!("{:?}", val)));
                  }

                  application_table.add_row(Row::new(row));
                }

                application_table.printstd();

                println!("\nbackend metrics:\n");

                for (ref key, ref values) in backend_data.iter() {
                  let mut row = Vec::new();
                  row.push(cell!(key));

                  for val in values.iter() {
                    row.push(cell!(format!("{:?}", val)));
                  }

                  backend_table.add_row(Row::new(row));
                }

                backend_table.printstd();
                break;
              }
            }
          }
        }
      }
    }
  }
}

pub fn add_application(channel: &mut Channel<ConfigMessage,ConfigMessageAnswer>, app_id: &str, sticky_session: bool) {
  order_command(channel, Order::AddApplication(Application {
    app_id:         String::from(app_id),
    sticky_session: sticky_session,
  }));
}

pub fn remove_application(channel: &mut Channel<ConfigMessage,ConfigMessageAnswer>, app_id: &str) {
  order_command(channel, Order::RemoveApplication(String::from(app_id)));
}

pub fn add_frontend(channel: &mut Channel<ConfigMessage,ConfigMessageAnswer>, app_id: &str, hostname: &str, path_begin: &str, certificate: Option<&str>) {
  if let Some(certificate_path) = certificate {
    match Config::load_file_bytes(certificate_path) {
      Ok(data) => {
        match calculate_fingerprint(&data) {
          Err(e)          => println!("could not calculate fingerprint for certificate: {:?}", e),
          Ok(fingerprint) => {
            order_command(channel, Order::AddHttpsFront(HttpsFront {
              app_id: String::from(app_id),
              hostname: String::from(hostname),
              path_begin: String::from(path_begin),
              fingerprint: CertFingerprint(fingerprint),
            }));
          },
        }
      },
      Err(e) => println!("could not load file: {:?}", e)
    }
  } else {
    order_command(channel, Order::AddHttpFront(HttpFront {
      app_id: String::from(app_id),
      hostname: String::from(hostname),
      path_begin: String::from(path_begin),
    }));
  }
}

pub fn remove_frontend(channel: &mut Channel<ConfigMessage,ConfigMessageAnswer>, app_id: &str, hostname: &str, path_begin: &str, certificate: Option<&str>) {
  if let Some(certificate_path) = certificate {
    match Config::load_file_bytes(certificate_path) {
      Ok(data) => {
        match calculate_fingerprint(&data) {
          Err(e)          => println!("could not calculate fingerprint for certificate: {:?}", e),
          Ok(fingerprint) => {
            order_command(channel, Order::RemoveHttpsFront(HttpsFront {
              app_id: String::from(app_id),
              hostname: String::from(hostname),
              path_begin: String::from(path_begin),
              fingerprint: CertFingerprint(fingerprint),
            }));
          },
        }
      },
      Err(e) => println!("could not load file: {:?}", e)
    }
  } else {
    order_command(channel, Order::RemoveHttpFront(HttpFront {
      app_id: String::from(app_id),
      hostname: String::from(hostname),
      path_begin: String::from(path_begin),
    }));
  }
}


pub fn add_backend(channel: &mut Channel<ConfigMessage,ConfigMessageAnswer>, app_id: &str, instance_id: &str, ip: &str, port: u16) {
  order_command(channel, Order::AddInstance(Instance {
      app_id: String::from(app_id),
      instance_id: String::from(instance_id),
      ip_address: String::from(ip),
      port: port
    }));
}

pub fn remove_backend(channel: &mut Channel<ConfigMessage,ConfigMessageAnswer>, app_id: &str, instance_id: &str, ip: &str, port: u16) {
    order_command(channel, Order::RemoveInstance(Instance {
      app_id: String::from(app_id),
      instance_id: String::from(instance_id),
      ip_address: String::from(ip),
      port: port
    }));
}

pub fn add_certificate(channel: &mut Channel<ConfigMessage,ConfigMessageAnswer>, certificate_path: &str, certificate_chain_path: &str, key_path: &str) {
  match Config::load_file(certificate_path) {
    Err(e) => println!("could not load certificate: {:?}", e),
    Ok(certificate) => {
      match Config::load_file(certificate_chain_path).map(split_certificate_chain) {
        Err(e) => println!("could not load certificate chain: {:?}", e),
        Ok(certificate_chain) => {
          match Config::load_file(key_path) {
            Err(e) => println!("could not load key: {:?}", e),
            Ok(key) => {
              order_command(channel, Order::AddCertificate(CertificateAndKey {
                certificate: certificate,
                certificate_chain: certificate_chain,
                key: key
              }));

            }
          }
        }
      }
    }
  }
}

pub fn remove_certificate(channel: &mut Channel<ConfigMessage,ConfigMessageAnswer>, certificate_path: &str) {
  match Config::load_file_bytes(certificate_path) {
    Ok(data) => {
      match calculate_fingerprint(&data) {
        Ok(fingerprint) => order_command(channel, Order::RemoveCertificate(CertFingerprint(fingerprint))),
        Err(e)          => println!("could not calculate finrprint for certificate: {:?}", e)
      }
    },
    Err(e) => println!("could not load file: {:?}", e)
  }
}

pub fn add_tcp_front(channel: &mut Channel<ConfigMessage,ConfigMessageAnswer>, app_id: &str, ip_address: &str, port: u16) {
  order_command(channel, Order::AddTcpFront(TcpFront {
    app_id: String::from(app_id),
    ip_address: String::from(ip_address),
    port: port
  }));
}

pub fn remove_tcp_front(channel: &mut Channel<ConfigMessage,ConfigMessageAnswer>, app_id: &str, ip_address: &str, port: u16) {
  order_command(channel, Order::RemoveTcpFront(TcpFront {
    app_id: String::from(app_id),
    ip_address: String::from(ip_address),
    port: port
  }));
}

pub fn query_application(channel: &mut Channel<ConfigMessage,ConfigMessageAnswer>, id: Option<&str>) {
  let command = match id {
    Some(app_id) => ConfigCommand::Query(Query::Application(app_id.to_string())),
    None         => ConfigCommand::Query(Query::Applications),
  };

  let id = generate_id();
  channel.write_message(&ConfigMessage::new(
    id.clone(),
    command,
    None,
  ));

  match channel.read_message() {
    None          => println!("the proxy didn't answer"),
    Some(message) => {
      if id != message.id {
        println!("received message with invalid id: {:?}", message);
        return;
      }
      match message.status {
        ConfigMessageStatus::Processing => {
          // do nothing here
          // for other messages, we would loop over read_message
          // until an error or ok message was sent
        },
        ConfigMessageStatus::Error => {
          println!("could not query proxy state: {}", message.message);
        },
        ConfigMessageStatus::Ok => {
          println!("Proxy config answer:\n{}\n{:#?}", message.message, message.data);
        }
      }
    }
  }
}

pub fn logging_filter(channel: &mut Channel<ConfigMessage,ConfigMessageAnswer>, filter: &str) {
  order_command(channel, Order::Logging(String::from(filter)));
}

fn order_command(channel: &mut Channel<ConfigMessage,ConfigMessageAnswer>, order: Order) {
  let id = generate_id();
  channel.write_message(&ConfigMessage::new(
    id.clone(),
    ConfigCommand::ProxyConfiguration(order.clone()),
    None,
  ));

  match channel.read_message() {
    None          => println!("the proxy didn't answer"),
    Some(message) => {
      if id != message.id {
        println!("received message with invalid id: {:?}", message);
        return;
      }
      match message.status {
        ConfigMessageStatus::Processing => {
          // do nothing here
          // for other messages, we would loop over read_message
          // until an error or ok message was sent
        },
        ConfigMessageStatus::Error => {
          println!("could not execute order: {}", message.message);
          exit(1);
        },
        ConfigMessageStatus::Ok => {
          //deactivate success messages for now
          /*
          match order {
            Order::AddApplication(_) => println!("application added : {}", message.message),
            Order::RemoveApplication(_) => println!("application removed : {} ", message.message),
            Order::AddInstance(_) => println!("backend added : {}", message.message),
            Order::RemoveInstance(_) => println!("backend removed : {} ", message.message),
            Order::AddCertificate(_) => println!("certificate added: {}", message.message),
            Order::RemoveCertificate(_) => println!("certificate removed: {}", message.message),
            Order::AddHttpFront(_) => println!("front added: {}", message.message),
            Order::RemoveHttpFront(_) => println!("front removed: {}", message.message),
            _ => {
              // do nothing for now
            }
          }
          */
        }
      }
    }
  }
}
