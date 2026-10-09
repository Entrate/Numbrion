//! Streaming input + a small thread pool. The reader thread feeds one fixture line at a time through
//! a bounded channel, so memory stays proportional to `jobs`, not to the input size.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, sync_channel};
use std::thread;

use flate2::read::MultiGzDecoder;

/// Which battles of the input to process (positions are 0-based, blank lines do not count).
#[derive(Clone, Debug, Default)]
pub struct Selection {
    pub skip: usize,
    pub limit: Option<usize>,
    /// Inclusive position ranges (`--battle 3,7,10-20`). `None` = all.
    pub only: Option<Vec<(usize, usize)>>,
}

impl Selection {
    pub fn parse_only(spec: &str) -> Result<Vec<(usize, usize)>, String> {
        let mut out = Vec::new();
        for part in spec.split(',').filter(|p| !p.is_empty()) {
            let (a, b) = match part.split_once('-') {
                Some((a, b)) => (a, b),
                None => (part, part),
            };
            let a: usize = a.trim().parse().map_err(|_| format!("bad battle spec {part:?}"))?;
            let b: usize = b.trim().parse().map_err(|_| format!("bad battle spec {part:?}"))?;
            if b < a {
                return Err(format!("bad range {part:?}"));
            }
            out.push((a, b));
        }
        if out.is_empty() {
            return Err("empty --battle spec".into());
        }
        Ok(out)
    }

    fn selected(&self, pos: usize) -> bool {
        if pos < self.skip {
            return false;
        }
        match &self.only {
            Some(r) => r.iter().any(|&(a, b)| pos >= a && pos <= b),
            None => true,
        }
    }

    fn max_pos(&self) -> Option<usize> {
        self.only.as_ref().and_then(|r| r.iter().map(|&(_, b)| b).max())
    }
}

/// Open a plain or gzipped (`.gz` extension or gzip magic bytes) file, or stdin for `-`.
pub fn open_input(path: &str) -> io::Result<Box<dyn BufRead + Send>> {
    let raw: Box<dyn Read + Send> = if path == "-" { Box::new(io::stdin()) } else { Box::new(File::open(path)?) };
    let mut br = BufReader::with_capacity(1 << 20, raw);
    let gz = {
        let head = br.fill_buf()?;
        head.len() >= 2 && head[0] == 0x1f && head[1] == 0x8b
    };
    Ok(if gz { Box::new(BufReader::with_capacity(1 << 20, MultiGzDecoder::new(br))) } else { Box::new(br) })
}

pub fn default_jobs() -> usize {
    thread::available_parallelism().map_or(4, |n| n.get())
}

/// Read the selected lines of `input`, run `work(pos, line)` on `jobs` threads and hand each result
/// to `sink` on the calling thread (completion order, not input order). Setting `stop` ends reading
/// early; queued jobs are dropped.
pub fn for_each_line<R: Send>(
    input: &str,
    sel: &Selection,
    jobs: usize,
    stop: &AtomicBool,
    work: impl Fn(usize, &str) -> R + Sync,
    mut sink: impl FnMut(usize, R),
) -> io::Result<()> {
    let reader = open_input(input)?;
    let jobs = jobs.max(1);
    let (job_tx, job_rx) = sync_channel::<(usize, String)>(jobs * 2);
    let job_rx = Mutex::new(job_rx);
    let (res_tx, res_rx) = channel::<(usize, R)>();
    let read_err: Mutex<Option<io::Error>> = Mutex::new(None);

    thread::scope(|s| {
        // Reader.
        let read_err = &read_err;
        s.spawn(move || {
            let mut reader = reader;
            let mut buf = String::new();
            let (mut pos, mut taken) = (0usize, 0usize);
            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                buf.clear();
                match reader.read_line(&mut buf) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(e) => {
                        *read_err.lock().unwrap() = Some(e);
                        break;
                    }
                }
                if buf.trim().is_empty() {
                    continue;
                }
                let p = pos;
                pos += 1;
                if !sel.selected(p) {
                    if sel.max_pos().is_some_and(|m| p >= m) {
                        break;
                    }
                    continue;
                }
                if sel.limit.is_some_and(|l| taken >= l) {
                    break;
                }
                taken += 1;
                let cap = buf.len();
                if job_tx.send((p, std::mem::replace(&mut buf, String::with_capacity(cap)))).is_err() {
                    break;
                }
                if sel.max_pos().is_some_and(|m| p >= m) {
                    break;
                }
            }
            // job_tx dropped here: workers see the end of input.
        });
        // Workers.
        let work = &work;
        let job_rx = &job_rx;
        for _ in 0..jobs {
            let res_tx = res_tx.clone();
            s.spawn(move || {
                loop {
                    let job = job_rx.lock().unwrap().recv();
                    let Ok((pos, line)) = job else { break };
                    if stop.load(Ordering::Relaxed) {
                        continue; // drain without working
                    }
                    let r = work(pos, &line);
                    if res_tx.send((pos, r)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(res_tx);
        for (pos, r) in res_rx {
            sink(pos, r);
        }
    });
    match read_err.into_inner().unwrap() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn selection_parse() {
        assert_eq!(Selection::parse_only("3,10-12").unwrap(), vec![(3, 3), (10, 12)]);
        assert!(Selection::parse_only("5-2").is_err());
        assert!(Selection::parse_only("x").is_err());
    }

    #[test]
    fn streams_selected_lines_in_parallel() {
        let dir = std::env::temp_dir().join(format!("difftest-runner-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lines.txt");
        let mut f = File::create(&path).unwrap();
        for i in 0..100 {
            writeln!(f, "line {i}").unwrap();
            if i % 10 == 0 {
                writeln!(f).unwrap(); // blank lines are ignored
            }
        }
        drop(f);
        let stop = AtomicBool::new(false);
        let run = |sel: &Selection, jobs| {
            let mut got = Vec::new();
            for_each_line(
                path.to_str().unwrap(),
                sel,
                jobs,
                &stop,
                |pos, l| (pos, l.trim_end().to_string()),
                |_, r| got.push(r),
            )
            .unwrap();
            got.sort();
            got
        };
        let all = run(&Selection::default(), 4);
        assert_eq!(all.len(), 100);
        assert_eq!(all[7], (7, "line 7".to_string()));
        let sel = Selection { skip: 10, limit: Some(5), only: None };
        assert_eq!(run(&sel, 3).iter().map(|x| x.0).collect::<Vec<_>>(), vec![10, 11, 12, 13, 14]);
        let sel = Selection { only: Some(Selection::parse_only("2,50-52").unwrap()), ..Default::default() };
        assert_eq!(run(&sel, 2).iter().map(|x| x.0).collect::<Vec<_>>(), vec![2, 50, 51, 52]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
