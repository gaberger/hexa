fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = habit_streak::run(
        &args,
        &|k| std::env::var(k).ok(),
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    );
    std::process::exit(code);
}
