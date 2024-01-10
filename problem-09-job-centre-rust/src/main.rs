use json::{self, JsonValue};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tokio::{
    io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::{mpsc, Mutex},
};

type JobId = u64;
type WorkerId = u64;

#[derive(Debug, Clone)]
enum Request {
    Put {
        queue: String,
        job: JsonValue,
        priority: u64,
    },
    Get {
        queues: Vec<String>,
        wait: bool,
    },
    Delete {
        id: JobId,
    },
    Abort {
        id: JobId,
    },
}

#[derive(Debug, Clone)]
struct State {
    queues: Arc<Mutex<HashMap<String, Queue>>>,
    job_queues: Arc<Mutex<HashMap<JobId, String>>>,
    current_job_id: Arc<AtomicU64>,
    current_worker_id: Arc<AtomicU64>,
}

impl State {
    fn new() -> Self {
        Self {
            queues: Arc::new(Mutex::new(HashMap::new())),
            job_queues: Arc::new(Mutex::new(HashMap::new())),
            current_job_id: Arc::new(AtomicU64::new(0)),
            current_worker_id: Arc::new(AtomicU64::new(0)),
        }
    }

    async fn put_job(&self, queue_name: String, job: Job) {
        {
            let mut job_queues = self.job_queues.lock().await;
            job_queues.insert(job.id, queue_name.clone());
        }
        {
            let mut queues = self.queues.lock().await;
            queues
                .entry(queue_name)
                .or_insert_with(|| Queue::new())
                .put(job)
                .await;
        }
    }

    fn get_job_with_highest_priority(
        queues: &HashMap<String, Queue>,
        queue_names: &[String],
    ) -> Option<Job> {
        queue_names
            .into_iter()
            .flat_map(|queue_name| queues.get(queue_name))
            .flat_map(|queue| queue.get_job_with_highest_priority())
            .max_by_key(|job| job.priority)
            .cloned()
    }

    fn claim_job(queues: &mut HashMap<String, Queue>, job: &Job, worker_id: WorkerId) {
        queues.get_mut(&job.queue).unwrap().claim(job.id, worker_id);
    }

    async fn get_job(&self, queue_names: &[String], worker_id: WorkerId) -> Option<Job> {
        let mut queues = self.queues.lock().await;

        let maybe_job = Self::get_job_with_highest_priority(&queues, queue_names).map(|job| {
            Self::claim_job(&mut queues, &job, worker_id);
            job
        });

        maybe_job
    }

    async fn get_job_wait(
        &self,
        queue_names: Vec<String>,
        worker_id: WorkerId,
    ) -> GetJobWaitResult {
        let mut queues = self.queues.lock().await;
        let maybe_job = Self::get_job_with_highest_priority(&queues, &queue_names);
        match maybe_job {
            Some(job) => {
                Self::claim_job(&mut queues, &job, worker_id);
                GetJobWaitResult::Job(job)
            }
            None => {
                let (sender, receiver) = mpsc::channel(1);
                queue_names.into_iter().for_each(|queue_name| {
                    queues
                        .entry(queue_name)
                        .or_insert_with(|| Queue::new())
                        .enqueue_claim(worker_id, sender.clone())
                });
                GetJobWaitResult::Wait(receiver)
            }
        }
    }

    async fn delete_job(&self, job_id: JobId) -> bool {
        let maybe_queue_name;
        {
            let mut job_queues = self.job_queues.lock().await;
            maybe_queue_name = job_queues.remove(&job_id);
        }

        match maybe_queue_name {
            None => false,
            Some(queue_name) => {
                let mut queues = self.queues.lock().await;
                match queues.get_mut(&queue_name) {
                    Some(queue) => queue.delete(job_id),
                    None => false,
                }
            }
        }
    }

    async fn abort_job(&self, job_id: JobId, worker_id: WorkerId) -> bool {
        let maybe_queue_name;
        {
            let job_queues = self.job_queues.lock().await;
            maybe_queue_name = job_queues.get(&job_id).cloned();
        }

        match maybe_queue_name {
            None => false,
            Some(queue_name) => {
                let mut queues = self.queues.lock().await;
                match queues.get_mut(&queue_name) {
                    Some(queue) => queue.abort(job_id, worker_id).await,
                    None => false,
                }
            }
        }
    }

    async fn handle_disconnect(&self, worker_id: WorkerId, claimed_job_ids: &HashSet<JobId>) {
        let mut clean_map: HashMap<String, Vec<JobId>> = HashMap::new();

        {
            let job_queues = self.job_queues.lock().await;
            claimed_job_ids.into_iter().for_each(|job_id| {
                if let Some(queue_name) = job_queues.get(job_id) {
                    clean_map
                        .entry(queue_name.clone())
                        .or_insert_with(|| vec![])
                        .push(*job_id);
                }
            });
        }

        {
            let mut queues = self.queues.lock().await;
            for (queue_name, job_ids) in clean_map {
                if let Some(queue) = queues.get_mut(&queue_name) {
                    for job_id in job_ids {
                        queue.abort(job_id, worker_id).await;
                    }
                }
            }
        }
    }

    fn next_job_id(&self) -> JobId {
        self.current_job_id.fetch_add(1, Ordering::Relaxed)
    }

    fn next_worker_id(&self) -> WorkerId {
        self.current_worker_id.fetch_add(1, Ordering::Relaxed)
    }
}

enum GetJobWaitResult {
    Job(Job),
    Wait(mpsc::Receiver<Job>),
}

#[derive(Debug)]
struct Queue {
    waiting_workers: VecDeque<(WorkerId, mpsc::Sender<Job>)>,
    jobs: HashMap<JobId, Job>,
}

impl Queue {
    fn new() -> Self {
        Self {
            waiting_workers: VecDeque::new(),
            jobs: HashMap::new(),
        }
    }

    async fn send_job(waiting_workers: &mut VecDeque<(WorkerId, mpsc::Sender<Job>)>, job: &mut Job) {
        while let Some((worker_id, sender)) = waiting_workers.pop_front() {
            if !sender.is_closed() {
                // Careful: Return of Ok does not guarantee that the value was received
                if let Ok(_) = sender.send(job.clone()).await {
                    job.worker = Some(worker_id);
                    break;
                }
            }
        }
    }

    async fn put(&mut self, mut job: Job) {
        Self::send_job(&mut self.waiting_workers, &mut job).await;
        self.jobs.insert(job.id, job);
    }

    fn get_job_with_highest_priority(&self) -> Option<&Job> {
        self.jobs
            .values()
            .filter(|job| job.worker.is_none())
            // Could be inefficient for longer queues, but implementing a heap isn't possible in a
            // simple way here
            .max_by_key(|job| job.priority)
    }

    fn claim(&mut self, job_id: JobId, worker_id: WorkerId) -> Option<Job> {
        self.jobs.get_mut(&job_id).map(|job| {
            job.worker = Some(worker_id);
            job.clone()
        })
    }

    fn enqueue_claim(&mut self, worker_id: WorkerId, sender: mpsc::Sender<Job>) {
        self.waiting_workers.push_back((worker_id, sender));
    }

    fn delete(&mut self, job_id: JobId) -> bool {
        self.jobs.remove(&job_id).is_some()
    }

    async fn abort(&mut self, job_id: JobId, worker_id: WorkerId) -> bool {
        let maybe_job = self.jobs
            .get_mut(&job_id)
            .filter(|job| job.worker == Some(worker_id));

        match maybe_job {
            Some(mut job) => {
                job.worker = None;
                Self::send_job(&mut self.waiting_workers, &mut job).await;
                true
            },
            None => false
        }
    }
}

#[derive(Debug, Clone)]
struct Job {
    id: JobId,
    content: JsonValue,
    priority: u64,
    queue: String,
    worker: Option<WorkerId>,
}

impl Job {
    fn new_with_unique_id(state: &State, content: JsonValue, priority: u64, queue: String) -> Self {
        Self {
            id: state.next_job_id(),
            content,
            priority,
            queue,
            worker: None,
        }
    }
}

#[derive(Debug)]
enum HandleRequestError {
    NoJob,
}

async fn handle_socket(mut socket: TcpStream, state: State) {
    let worker_id = state.next_worker_id();
    let mut claimed_job_ids: HashSet<JobId> = HashSet::new();

    let (read_half, mut write_half) = socket.split();
    let mut lines = BufReader::new(read_half).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        match json::parse_json(&line) {
            Ok(json_val) => {
                let request = match parse_request(&json_val) {
                    Ok(req) => req,
                    Err(msg) => {
                        if let Err(_) = send_error_response(&mut write_half, msg).await {
                            break;
                        }
                        continue;
                    }
                };

                let response_json =
                    match handle_request(request, worker_id, &state, &mut claimed_job_ids).await {
                        Ok(response_json) => response_json,
                        Err(HandleRequestError::NoJob) => {
                            if let Err(_) = send_no_job_response(&mut write_half).await {
                                break;
                            }
                            continue;
                        }
                    };

                let response = format!("{}\n", response_json);
                if let Err(_) = write_half.write_all(response.as_bytes()).await {
                    break;
                }
            }
            _ => {
                if let Err(_) = send_error_response(&mut write_half, "invalid request").await {
                    break;
                }
            }
        }
    }

    state.handle_disconnect(worker_id, &claimed_job_ids).await;
}

async fn handle_request(
    request: Request,
    worker_id: WorkerId,
    state: &State,
    claimed_job_ids: &mut HashSet<JobId>,
) -> Result<JsonValue, HandleRequestError> {
    // Could instead return some kind of response object, that the caller can transform into json.
    // This would probably improve testability
    match request {
        Request::Put {
            queue,
            job: job_content,
            priority,
        } => {
            let job = Job::new_with_unique_id(state, job_content, priority, queue.clone());
            let job_id = job.id;
            state.put_job(queue, job).await;

            let response = JsonValue::Object(HashMap::from([
                ("status".to_string(), JsonValue::String("ok".to_string())),
                ("id".to_string(), JsonValue::Number(job_id as f64)),
            ]));
            Ok(response)
        }
        Request::Get { queues, wait } => {
            let maybe_job = if wait {
                match state.get_job_wait(queues, worker_id).await {
                    GetJobWaitResult::Job(job) => Some(job),
                    GetJobWaitResult::Wait(mut receiver) => receiver.recv().await,
                }
            } else {
                state.get_job(&queues, worker_id).await
            };

            match maybe_job {
                Some(job) => {
                    claimed_job_ids.insert(job.id);
                    let response = JsonValue::Object(HashMap::from([
                        ("status".to_string(), JsonValue::String("ok".to_string())),
                        ("id".to_string(), JsonValue::Number(job.id as f64)),
                        ("job".to_string(), job.content),
                        ("pri".to_string(), JsonValue::Number(job.priority as f64)),
                        ("queue".to_string(), JsonValue::String(job.queue)),
                    ]));
                    Ok(response)
                }
                None => Err(HandleRequestError::NoJob),
            }
        }
        Request::Delete { id } => {
            let deleted = state.delete_job(id).await;

            if !deleted {
                Err(HandleRequestError::NoJob)
            } else {
                claimed_job_ids.remove(&id);
                let response = JsonValue::Object(HashMap::from([(
                    "status".to_string(),
                    JsonValue::String("ok".to_string()),
                )]));
                Ok(response)
            }
        }
        Request::Abort { id } => {
            let aborted = state.abort_job(id, worker_id).await;

            if !aborted {
                Err(HandleRequestError::NoJob)
            } else {
                claimed_job_ids.remove(&id);
                let response = JsonValue::Object(HashMap::from([(
                    "status".to_string(),
                    JsonValue::String("ok".to_string()),
                )]));
                Ok(response)
            }
        }
    }
}

fn parse_request(json_val: &JsonValue) -> Result<Request, String> {
    if let JsonValue::Object(json_obj) = json_val {
        match json_obj.get("request") {
            Some(JsonValue::String(request)) => match request.as_ref() {
                "put" => {
                    let queue = match json_obj.get("queue") {
                        Some(JsonValue::String(queue)) => queue,
                        _ => return Err("put request is missing string queue field".to_string()),
                    };

                    let job = match json_obj.get("job") {
                        Some(json_obj @ JsonValue::Object(_)) => json_obj,
                        _ => return Err("put request is missing json object job field".to_string()),
                    };

                    let priority = match json_obj.get("pri") {
                        Some(JsonValue::Number(priority)) => parse_number(*priority)?,
                        _ => return Err("put request is missing number pri field".to_string()),
                    };

                    Ok(Request::Put {
                        queue: queue.to_string(),
                        job: job.clone(),
                        priority,
                    })
                }
                "get" => {
                    let queues = match json_obj.get("queues") {
                        Some(JsonValue::Array(queue_json_vals)) => {
                            let mut queues = Vec::with_capacity(queue_json_vals.len());
                            for json_val in queue_json_vals {
                                match json_val {
                                    JsonValue::String(queue) => queues.push(queue.clone()),
                                    _ => return Err(
                                        "queues field in get request has to contain only strings"
                                            .to_string(),
                                    ),
                                }
                            }
                            queues
                        }
                        _ => return Err("get request is missing array queues field".to_string()),
                    };

                    let wait = match json_obj.get("wait") {
                        Some(JsonValue::Boolean(wait)) => *wait,
                        // Maybe has to throw error when wait field is in request but not bool; not
                        // specified in problem description
                        _ => false,
                    };

                    Ok(Request::Get { queues, wait })
                }
                "delete" => {
                    let id = match json_obj.get("id") {
                        Some(JsonValue::Number(id)) => parse_number(*id)?,
                        _ => return Err("delete request is missing integer id field".to_string()),
                    };
                    Ok(Request::Delete { id })
                }
                "abort" => {
                    let id = match json_obj.get("id") {
                        Some(JsonValue::Number(id)) => parse_number(*id)?,
                        _ => return Err("abort request is missing integer id field".to_string()),
                    };
                    Ok(Request::Abort { id })
                }
                _ => Err("invalid request field".to_string()),
            },
            _ => Err("request is missing field 'request'".to_string()),
        }
    } else {
        Err("invalid json request".to_string())
    }
}

fn parse_number(number: f64) -> Result<u64, String> {
    if !number.is_finite() {
        Err("number isn't finite".to_string())
    } else if number < 0.0 {
        Err("number is negative".to_string())
    } else if (number.round() - number).abs() > 1e-9 {
        Err("number has to be an integer".to_string())
    } else {
        Ok(number as u64)
    }
}

async fn send_error_response(
    writer: &mut (impl AsyncWriteExt + Unpin),
    error_text: impl Into<String>,
) -> io::Result<()> {
    let json_val = JsonValue::Object(HashMap::from([
        ("status".to_string(), JsonValue::String("error".to_string())),
        ("error".to_string(), JsonValue::String(error_text.into())),
    ]));
    let response = format!("{}\n", json_val);
    writer.write_all(response.as_bytes()).await
}

async fn send_no_job_response(writer: &mut (impl AsyncWriteExt + Unpin)) -> io::Result<()> {
    let json_val = JsonValue::Object(HashMap::from([(
        "status".to_string(),
        JsonValue::String("no-job".to_string()),
    )]));
    let response = format!("{}\n", json_val);
    writer.write_all(response.as_bytes()).await
}

#[tokio::main]
async fn main() {
    let state = State::new();

    let server = TcpListener::bind("0.0.0.0:10000").await.expect("bind");
    loop {
        if let Ok((socket, _addr)) = server.accept().await {
            tokio::spawn(handle_socket(socket, state.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn test_implicit_abort() {
        // Tests whether a waiting client gets a claimed job, when the client that works on that
        // job aborts it via disconnecting

        let state = State::new();

        let put_request = Request::Put {
            queue: "some-queue".to_string(),
            job: JsonValue::Object(HashMap::from([(
                "name".to_string(),
                JsonValue::String("some name".to_string()),
            )])),
            priority: 100,
        };

        let response = handle_request(put_request, 0, &state, &mut HashSet::new())
            .await
            .unwrap();

        let job_id = if let JsonValue::Object(obj) = response {
            if let JsonValue::Number(num) = obj.get("id").unwrap() {
                *num as JobId
            } else {
                unreachable!()
            }
        } else {
            unreachable!();
        };

        let get_request = Request::Get {
            queues: vec!["some-queue".to_string(), "other-queue".to_string()],
            wait: true,
        };

        let first_worker_id = 1;
        let second_worker_id = 2;

        let first_task = tokio::spawn({
            let state = state.clone();
            let get_request = get_request.clone();
            let mut claimed_job_ids = HashSet::new();
            async move {
                handle_request(get_request, first_worker_id, &state, &mut claimed_job_ids).await
            }
        });

        let second_task = tokio::spawn({
            let state = state.clone();
            let get_request = get_request.clone();
            let mut claimed_job_ids = HashSet::new();
            async move {
                handle_request(get_request, second_worker_id, &state, &mut claimed_job_ids).await
            }
        });

        first_task.await.unwrap().unwrap();

        state
            .handle_disconnect(first_worker_id, &HashSet::from([job_id]))
            .await;

        assert!(tokio::time::timeout(Duration::from_millis(100), second_task).await.is_ok());
    }
}
