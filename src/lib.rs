//! proof — заявление с доказательством, которое можно перепроверить.
//!
//! Проблема, которую это лечит, настоящая: в переписке агентов и людей «работает»
//! означает «я запустил и вот вывод». Через день это утверждение нельзя проверить:
//! вывод остался в переписке, команда забыта, а репозиторий уехал. Здесь заявление
//! хранится вместе с **точной командой** и **отпечатком вывода**, и любой следующий
//! участник перепроверяет его одной командой — не веря на слово.
//!
//! Три вердикта, и они честные:
//!
//! * `verified` — команда вернула ожидаемый код, вывод тот же;
//! * `moved` — команда работает, но вывод изменился (например, тестов стало 82 вместо
//!   80): заявление устарело, а не сломалось;
//! * `broken` — команда вернула не тот код: доказательство перестало быть доказательством.
//!
//! Отпечаток считается по **нормализованному** выводу: числа-тайминги (`0.12s`)
//! заменяются на `<t>`, иначе любой прогон тестов давал бы «moved» из-за миллисекунд.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Status {
    /// Ещё не запускали.
    Never,
    /// Верно и подтверждено.
    Verified,
    /// Работает, но вывод изменился — заявление устарело.
    Moved,
    /// Команда больше не возвращает ожидаемое.
    Broken,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Never => "never",
            Status::Verified => "verified",
            Status::Moved => "moved",
            Status::Broken => "broken",
        }
    }
    pub fn parse(text: &str) -> Status {
        match text {
            "verified" => Status::Verified,
            "moved" => Status::Moved,
            "broken" => Status::Broken,
            _ => Status::Never,
        }
    }
    pub fn ru(&self) -> &'static str {
        match self {
            Status::Never => "не проверялось",
            Status::Verified => "подтверждено",
            Status::Moved => "устарело",
            Status::Broken => "сломано",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Claim {
    pub id: u32,
    /// Что утверждается: «80 тестов зелёные».
    pub claim: String,
    /// Чем это проверяется: точная команда.
    pub cmd: String,
    pub dir: String,
    pub by: String,
    pub expect: i32,
    pub made: u64,
    pub runs: u32,
    pub status: Status,
    pub last_run: u64,
    pub digest: String,
    pub excerpt: String,
}

impl Claim {
    pub fn new(
        id: u32,
        claim: &str,
        cmd: &str,
        dir: &str,
        by: &str,
        expect: i32,
        made: u64,
    ) -> Claim {
        Claim {
            id,
            claim: claim.to_string(),
            cmd: cmd.to_string(),
            dir: dir.to_string(),
            by: by.to_string(),
            expect,
            made,
            runs: 0,
            status: Status::Never,
            last_run: 0,
            digest: String::new(),
            excerpt: String::new(),
        }
    }

    pub fn render(&self) -> String {
        format!(
            "id={}\nclaim={}\ncmd={}\ndir={}\nby={}\nexpect={}\nmade={}\nruns={}\nstatus={}\nlast_run={}\ndigest={}\nexcerpt={}\n",
            self.id,
            self.claim,
            self.cmd,
            self.dir,
            self.by,
            self.expect,
            self.made,
            self.runs,
            self.status.as_str(),
            self.last_run,
            self.digest,
            self.excerpt.replace('\n', "⏎")
        )
    }

    pub fn parse(text: &str) -> Option<Claim> {
        let mut claim = Claim::new(0, "", "", "", "", 0, 0);
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "id" => claim.id = value.parse().unwrap_or(0),
                "claim" => claim.claim = value.to_string(),
                "cmd" => claim.cmd = value.to_string(),
                "dir" => claim.dir = value.to_string(),
                "by" => claim.by = value.to_string(),
                "expect" => claim.expect = value.parse().unwrap_or(0),
                "made" => claim.made = value.parse().unwrap_or(0),
                "runs" => claim.runs = value.parse().unwrap_or(0),
                "status" => claim.status = Status::parse(value),
                "last_run" => claim.last_run = value.parse().unwrap_or(0),
                "digest" => claim.digest = value.to_string(),
                "excerpt" => claim.excerpt = value.replace('⏎', "\n"),
                _ => {}
            }
        }
        if claim.id == 0 || claim.claim.is_empty() {
            return None;
        }
        Some(claim)
    }
}

/// Отпечаток: FNV-1a 64. Не криптография — сравнение выводов, и только.
pub fn digest(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Убрать из вывода то, что меняется от прогона к прогону и ничего не значит.
pub fn normalize(output: &str) -> String {
    let mut out = String::with_capacity(output.len());
    for line in output.lines() {
        let mut first = true;
        for token in line.split_whitespace() {
            if !first {
                out.push(' ');
            }
            first = false;
            out.push_str(&stable_token(token));
        }
        out.push('\n');
    }
    out
}

/// `0.012s`, `1.38s`, `12s` → `<t>`; остальное не трогаем.
fn stable_token(token: &str) -> String {
    let trimmed = token.trim_end_matches([',', ';', ')', ':']);
    let suffix = &token[trimmed.len()..];
    if let Some(number) = trimmed.strip_suffix('s') {
        if !number.is_empty() && number.parse::<f64>().is_ok() {
            return format!("<t>{suffix}");
        }
    }
    token.to_string()
}

#[derive(Debug, Clone)]
pub struct RunResult {
    pub code: i32,
    pub digest: String,
    pub excerpt: String,
}

/// Запустить команду заявления. Это единственное место, где что-то исполняется.
pub fn run(claim: &Claim) -> Result<RunResult, String> {
    let mut command = Command::new("sh");
    command.arg("-c").arg(&claim.cmd);
    if !claim.dir.is_empty() {
        command.current_dir(&claim.dir);
    }
    let output = command
        .output()
        .map_err(|err| format!("не запустить «{}»: {err}", claim.cmd))?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    let errors = String::from_utf8_lossy(&output.stderr).to_string();
    if !errors.trim().is_empty() {
        text.push_str("\n--- stderr ---\n");
        text.push_str(&errors);
    }
    let normalized = normalize(&text);
    let tail: Vec<&str> = normalized.lines().rev().take(3).collect();
    let excerpt = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
    Ok(RunResult {
        code: output.status.code().unwrap_or(-1),
        digest: digest(&normalized),
        excerpt,
    })
}

/// Вердикт: код совпал — работает; вывод тот же — подтверждено.
pub fn judge(claim: &Claim, result: &RunResult) -> Status {
    if result.code != claim.expect {
        return Status::Broken;
    }
    if claim.digest.is_empty() || claim.digest == result.digest {
        Status::Verified
    } else {
        Status::Moved
    }
}

/// Применить прогон к заявлению, сохранив историю.
pub fn apply(claim: &mut Claim, result: &RunResult, at: u64) {
    claim.status = judge(claim, result);
    claim.runs += 1;
    claim.last_run = at;
    claim.digest = result.digest.clone();
    claim.excerpt = result.excerpt.clone();
}

/// Прогнать и записать в заявление одной операцией.
pub fn check_now(claim: &mut Claim) -> Result<Status, String> {
    let result = run(claim)?;
    apply(claim, &result, now());
    Ok(claim.status)
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Реестр заявлений — каталог `proofs/` с одним файлом на заявление.
pub struct Ledger {
    root: PathBuf,
}

impl Ledger {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Ledger> {
        let root = path.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        Ok(Ledger { root })
    }

    pub fn claims(&self) -> Vec<Claim> {
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(&self.root) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("proof") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            if let Some(claim) = Claim::parse(&text) {
                out.push(claim);
            }
        }
        out.sort_by_key(|claim| claim.id);
        out
    }

    pub fn next_id(&self) -> u32 {
        self.claims()
            .iter()
            .map(|claim| claim.id)
            .max()
            .unwrap_or(0)
            + 1
    }

    pub fn put(&self, claim: &Claim) -> io::Result<()> {
        fs::write(
            self.root.join(format!("{:04}.proof", claim.id)),
            claim.render(),
        )
    }

    pub fn get(&self, id: u32) -> Option<Claim> {
        self.claims().into_iter().find(|claim| claim.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("proof-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn timing_numbers_do_not_change_the_digest() {
        let a = normalize("Ran 80 tests in 0.012s\nOK\n");
        let b = normalize("Ran 80 tests in 1.38s\nOK\n");
        assert_eq!(digest(&a), digest(&b), "миллисекунды — не изменение");
        let c = normalize("Ran 82 tests in 1.40s\nOK\n");
        assert_ne!(digest(&a), digest(&c), "82 вместо 80 — это изменение");
    }

    #[test]
    fn green_command_is_verified_and_recheck_keeps_it() {
        let dir = temp("green");
        let ledger = Ledger::open(dir.join("proofs")).unwrap();
        let mut claim = Claim::new(
            ledger.next_id(),
            "скрипт возвращает 0",
            "true",
            dir.to_str().unwrap(),
            "forgessa",
            0,
            now(),
        );
        check_now(&mut claim).unwrap();
        assert_eq!(claim.status, Status::Verified);
        ledger.put(&claim).unwrap();
        let loaded = ledger.get(1).unwrap();
        let second = judge(&loaded, &run(&loaded).unwrap());
        assert_eq!(second, Status::Verified, "перепроверка подтверждает");
        assert_eq!(loaded.runs, 1, "счётчик прогонов хранится");
    }

    #[test]
    fn broken_when_the_command_stops_returning_the_expected_code() {
        let dir = temp("broken");
        let mut claim = Claim::new(1, "падает", "exit 3", dir.to_str().unwrap(), "x", 0, now());
        check_now(&mut claim).unwrap();
        assert_eq!(
            claim.status,
            Status::Broken,
            "код не тот — доказательства нет"
        );
    }

    #[test]
    fn moved_when_output_changes_but_the_command_works() {
        let dir = temp("moved");
        let file = dir.join("value.txt");
        fs::write(&file, "80").unwrap();
        let mut claim = Claim::new(
            1,
            "в файле 80",
            &format!("cat {}", file.display()),
            dir.to_str().unwrap(),
            "x",
            0,
            now(),
        );
        check_now(&mut claim).unwrap();
        assert_eq!(claim.status, Status::Verified);
        fs::write(&file, "82").unwrap();
        let verdict = judge(&claim, &run(&claim).unwrap());
        assert_eq!(
            verdict,
            Status::Moved,
            "вывод изменился, заявление устарело"
        );
        assert_eq!(
            claim.status,
            Status::Verified,
            "статус меняется только apply"
        );
    }

    #[test]
    fn expected_nonzero_exit_is_a_legitimate_proof() {
        let dir = temp("expect");
        let mut claim = Claim::new(
            1,
            "проверка падает как надо",
            "exit 1",
            dir.to_str().unwrap(),
            "x",
            1,
            now(),
        );
        check_now(&mut claim).unwrap();
        assert_eq!(claim.status, Status::Verified, "ожидали 1 — получили 1");
    }

    #[test]
    fn claim_survives_roundtrip_with_multiline_excerpt() {
        let mut claim = Claim::new(3, "три теста", "true", "/tmp", "анна", 0, 1000);
        claim.excerpt = "первая\nвторая\nтретья".to_string();
        claim.status = Status::Moved;
        claim.runs = 4;
        let back = Claim::parse(&claim.render()).unwrap();
        assert_eq!(back.id, 3);
        assert_eq!(back.claim, "три теста");
        assert_eq!(back.status, Status::Moved);
        assert_eq!(back.runs, 4);
        assert_eq!(back.excerpt, "первая\nвторая\nтретья");
    }

    #[test]
    fn ledger_numbers_claims_sequentially() {
        let dir = temp("ids");
        let ledger = Ledger::open(dir.join("proofs")).unwrap();
        assert_eq!(ledger.next_id(), 1);
        ledger
            .put(&Claim::new(1, "первое", "true", "", "x", 0, 0))
            .unwrap();
        ledger
            .put(&Claim::new(2, "второе", "true", "", "x", 0, 0))
            .unwrap();
        assert_eq!(ledger.next_id(), 3);
        assert_eq!(ledger.claims().len(), 2);
        assert_eq!(ledger.claims()[0].claim, "первое");
    }

    #[test]
    fn excerpt_keeps_the_tail_of_the_output() {
        let dir = temp("excerpt");
        let claim = Claim::new(
            1,
            "хвост",
            "printf 'a\\nb\\nc\\nd\\n'",
            dir.to_str().unwrap(),
            "x",
            0,
            now(),
        );
        let result = run(&claim).unwrap();
        assert!(
            result.excerpt.contains("d"),
            "в хвосте последняя строка: {}",
            result.excerpt
        );
        assert!(
            !result.excerpt.contains("a\n"),
            "начало не нужно: {}",
            result.excerpt
        );
    }
}
