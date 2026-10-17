//! `proof` — командная строка реестра заявлений.
//!
//! ```text
//! proof add --claim "80 тестов зелёные" --cmd "python3 -m unittest discover -s tests -t ." --dir repo
//! proof check          # перепроверить всё, что записано
//! proof list
//! proof show --id 1
//! proof demo
//! ```

use std::path::PathBuf;

use proof::{apply, judge, now, run, Claim, Ledger, Status};

struct Opts {
    args: Vec<String>,
}

impl Opts {
    fn parse(args: &[String]) -> Opts {
        Opts {
            args: args.to_vec(),
        }
    }
    fn flag(&self, name: &str) -> Option<&str> {
        let spelled = format!("--{name}");
        self.args
            .iter()
            .position(|item| item.as_str() == spelled.as_str())
            .and_then(|index| self.args.get(index + 1))
            .map(String::as_str)
    }
    fn number(&self, name: &str) -> Option<i64> {
        self.flag(name).and_then(|value| value.trim().parse().ok())
    }
    fn ledger(&self) -> PathBuf {
        PathBuf::from(self.flag("ledger").unwrap_or("proofs"))
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("add") => cmd_add(&args[1..]),
        Some("check") => cmd_check(&args[1..]),
        Some("list") => cmd_list(&args[1..]),
        Some("show") => cmd_show(&args[1..]),
        Some("demo") => cmd_demo(),
        Some("--help") | Some("-h") | None => {
            print_help();
            0
        }
        Some(other) => {
            eprintln!("неизвестная команда: {other}\n");
            print_help();
            2
        }
    };
    std::process::exit(code);
}

fn print_help() {
    println!(
        "proof — заявление с доказательством, которое можно перепроверить

  add    --claim \"что утверждается\" --cmd \"чем проверяется\"
         [--dir PATH] [--by КТО] [--expect КОД] [--ledger DIR]
  check  [--id N] [--ledger DIR]     перепроверить и показать вердикт
  list   [--ledger DIR]
  show   --id N [--ledger DIR]
  demo

Вердикты:
  подтверждено — команда вернула ожидаемый код, вывод тот же
  устарело     — работает, но вывод изменился: заявление надо переписать
  сломано      — код не тот: доказательство перестало быть доказательством

Код выхода check: 0 всё подтверждено или устарело, 1 есть сломанное, 2 ошибка."
    );
}

fn cmd_add(args: &[String]) -> i32 {
    let opts = Opts::parse(args);
    let (Some(text), Some(cmd)) = (opts.flag("claim"), opts.flag("cmd")) else {
        eprintln!("нужны --claim и --cmd");
        return 2;
    };
    let ledger = match Ledger::open(opts.ledger()) {
        Ok(ledger) => ledger,
        Err(err) => {
            eprintln!("не открыть реестр: {err}");
            return 2;
        }
    };
    let dir = opts
        .flag("dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("."));
    let mut claim = Claim::new(
        ledger.next_id(),
        text,
        cmd,
        dir.to_str().unwrap_or("."),
        opts.flag("by").unwrap_or("неизвестно"),
        opts.number("expect").unwrap_or(0) as i32,
        now(),
    );
    let result = match run(&claim) {
        Ok(result) => result,
        Err(err) => {
            eprintln!("{err}");
            return 2;
        }
    };
    apply(&mut claim, &result, now());
    if let Err(err) = ledger.put(&claim) {
        eprintln!("не записать заявление: {err}");
        return 2;
    }
    println!(
        "#{} {} — «{}» ({} прогон, код {})",
        claim.id,
        claim.status.ru(),
        claim.claim,
        claim.runs,
        result.code
    );
    if claim.status == Status::Broken {
        println!(
            "  но команда вернула {} вместо ожидаемых {}",
            result.code, claim.expect
        );
        return 1;
    }
    0
}

fn cmd_check(args: &[String]) -> i32 {
    let opts = Opts::parse(args);
    let ledger = match Ledger::open(opts.ledger()) {
        Ok(ledger) => ledger,
        Err(err) => {
            eprintln!("не открыть реестр: {err}");
            return 2;
        }
    };
    let claims: Vec<Claim> = match opts.number("id") {
        Some(id) => match ledger.get(id as u32) {
            Some(claim) => vec![claim],
            None => {
                eprintln!("заявления #{id} нет");
                return 2;
            }
        },
        None => ledger.claims(),
    };
    if claims.is_empty() {
        println!("реестр пуст: доказывать нечего");
        return 0;
    }
    let mut broken = 0;
    let mut moved = 0;
    let mut verified = 0;
    for mut claim in claims {
        let result = match run(&claim) {
            Ok(result) => result,
            Err(err) => {
                eprintln!("#{}: {err}", claim.id);
                broken += 1;
                continue;
            }
        };
        let was = claim.status;
        apply(&mut claim, &result, now());
        match claim.status {
            Status::Verified => verified += 1,
            Status::Moved => moved += 1,
            Status::Broken | Status::Never => broken += 1,
        }
        println!(
            "#{:<3} {:<14} {:<44} {}",
            claim.id,
            claim.status.ru(),
            truncate(&claim.claim, 44),
            if was == claim.status {
                String::new()
            } else {
                format!("(было: {})", was.ru())
            }
        );
        if claim.status != Status::Verified {
            for line in claim.excerpt.lines().take(3) {
                println!("       │ {line}");
            }
        }
        let _ = ledger.put(&claim);
    }
    println!(
        "\nитог: подтверждено {}, устарело {}, сломано {}",
        verified, moved, broken
    );
    if broken > 0 {
        1
    } else {
        0
    }
}

fn cmd_list(args: &[String]) -> i32 {
    let opts = Opts::parse(args);
    let ledger = match Ledger::open(opts.ledger()) {
        Ok(ledger) => ledger,
        Err(err) => {
            eprintln!("не открыть реестр: {err}");
            return 2;
        }
    };
    let claims = ledger.claims();
    if claims.is_empty() {
        println!("реестр пуст");
        return 0;
    }
    println!(
        "{:<4} {:<14} {:<10} {:<8} {}",
        "id", "вердикт", "прогонов", "возраст", "заявление"
    );
    let current = now();
    for claim in claims {
        println!(
            "{:<4} {:<14} {:<10} {:<8} {}",
            claim.id,
            claim.status.ru(),
            claim.runs,
            if claim.last_run == 0 {
                "—".to_string()
            } else {
                human_age(current.saturating_sub(claim.last_run))
            },
            truncate(&claim.claim, 50)
        );
    }
    0
}

fn cmd_show(args: &[String]) -> i32 {
    let opts = Opts::parse(args);
    let Some(id) = opts.number("id") else {
        eprintln!("нужен --id N");
        return 2;
    };
    let ledger = match Ledger::open(opts.ledger()) {
        Ok(ledger) => ledger,
        Err(err) => {
            eprintln!("не открыть реестр: {err}");
            return 2;
        }
    };
    let Some(claim) = ledger.get(id as u32) else {
        eprintln!("заявления #{id} нет");
        return 2;
    };
    println!("#{} «{}»", claim.id, claim.claim);
    println!(
        "  вердикт:   {} (прогонов {})",
        claim.status.ru(),
        claim.runs
    );
    println!("  кто:       {}", claim.by);
    println!("  где:       {}", claim.dir);
    println!("  команда:   {}", claim.cmd);
    println!("  ожидался:  код {}", claim.expect);
    println!("  отпечаток: {}", claim.digest);
    if !claim.excerpt.trim().is_empty() {
        println!("  хвост вывода:");
        for line in claim.excerpt.lines() {
            println!("    {line}");
        }
    }
    0
}

fn human_age(secs: u64) -> String {
    if secs >= 86_400 {
        format!("{}д", secs / 86_400)
    } else if secs >= 3600 {
        format!("{}ч", secs / 3600)
    } else if secs >= 60 {
        format!("{}м", secs / 60)
    } else {
        format!("{secs}с")
    }
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    text.chars()
        .take(width.saturating_sub(1))
        .collect::<String>()
        + "…"
}

// --- демонстрация -----------------------------------------------------------------
fn cmd_demo() -> i32 {
    let mut checks = 0;
    let mut fails = 0;
    let mut check = |name: &str, ok: bool, detail: String| {
        checks += 1;
        if !ok {
            fails += 1;
        }
        println!("  {} {name} — {detail}", if ok { "✓" } else { "✗" });
    };

    let dir = std::env::temp_dir().join(format!("proof-demo-{}", now()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ledger = Ledger::open(dir.join("proofs")).unwrap();
    let script = dir.join("suite.sh");
    std::fs::write(&script, "echo 'Ran 4 tests in 0.010s'; echo OK; exit 0\n").unwrap();

    // заявление, которое подтверждается
    let mut green = Claim::new(
        ledger.next_id(),
        "4 теста зелёные",
        &format!("sh {}", script.display()),
        dir.to_str().unwrap(),
        "forgessa",
        0,
        now(),
    );
    proof::check_now(&mut green).unwrap();
    ledger.put(&green).unwrap();
    check(
        "заявление записано с командой и отпечатком",
        green.status == Status::Verified && !green.digest.is_empty(),
        format!("отпечаток {}", &green.digest[..8]),
    );

    // перепроверка того же — подтверждает
    let again = judge(
        &ledger.get(1).unwrap(),
        &run(&ledger.get(1).unwrap()).unwrap(),
    );
    check(
        "перепроверка подтверждает, а не требует поверить",
        again == Status::Verified,
        "вывод тот же после нормализации".to_string(),
    );

    // вывод изменился: тестов стало больше
    std::fs::write(&script, "echo 'Ran 5 tests in 0.014s'; echo OK; exit 0\n").unwrap();
    let moved = judge(
        &ledger.get(1).unwrap(),
        &run(&ledger.get(1).unwrap()).unwrap(),
    );
    check(
        "число тестов выросло — вердикт «устарело»",
        moved == Status::Moved,
        "не «сломано»: команда работает".to_string(),
    );

    // команда упала
    std::fs::write(
        &script,
        "echo 'Ran 5 tests in 0.014s'; echo FAILED; exit 1\n",
    )
    .unwrap();
    let broken = judge(
        &ledger.get(1).unwrap(),
        &run(&ledger.get(1).unwrap()).unwrap(),
    );
    check(
        "команда упала — вердикт «сломано»",
        broken == Status::Broken,
        "доказательство перестало быть доказательством".to_string(),
    );

    // второе заявление: ожидаемо падающая проверка — законное доказательство
    let mut expected = Claim::new(
        ledger.next_id(),
        "проверка ловит регресс",
        &format!("sh {} ; test $? -ne 0", script.display()),
        dir.to_str().unwrap(),
        "forgessa",
        0,
        now(),
    );
    proof::check_now(&mut expected).unwrap();
    ledger.put(&expected).unwrap();
    check(
        "ожидаемое падение — тоже доказательство",
        expected.status == Status::Verified,
        "проверка, которая должна ловить регресс, его ловит".to_string(),
    );

    check(
        "реестр хранит историю прогонов",
        ledger.claims().len() == 2 && green.runs == 1,
        format!("заявлений {}", ledger.claims().len()),
    );

    println!("\nитог: {checks} проверок, {fails} провалов");
    let _ = std::fs::remove_dir_all(&dir);
    if fails == 0 {
        0
    } else {
        1
    }
}
