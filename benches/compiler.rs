use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use soppo::build;
use soppo::syntax::{FileId, Parser};

fn load_fixtures() -> Vec<(&'static str, String)> {
    let fixtures = [
        // Examples
        (
            "guessing_game",
            include_str!("../examples/guessing_game.sop"),
        ),
        (
            "file_processor",
            include_str!("../examples/file_processor.sop"),
        ),
        ("http_server", include_str!("../examples/http_server.sop")),
        ("todo_cli", include_str!("../examples/todo_cli.sop")),
        // Fixtures
        (
            "basic_go",
            include_str!("../tests/fixtures/single/pass/basic_go.sop"),
        ),
    ];

    fixtures
        .into_iter()
        .map(|(name, content)| (name, content.to_string()))
        .collect()
}

fn bench_parse(c: &mut Criterion) {
    let fixtures = load_fixtures();

    let mut group = c.benchmark_group("parse");
    for (name, source) in &fixtures {
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(BenchmarkId::new("file", name), source, |b, source| {
            b.iter(|| {
                let mut parser = Parser::new(source, FileId(0));
                parser.parse_file()
            });
        });
    }
    group.finish();
}

fn bench_typecheck(c: &mut Criterion) {
    let fixtures = load_fixtures();

    let mut group = c.benchmark_group("typecheck");
    for (name, source) in &fixtures {
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(BenchmarkId::new("file", name), source, |b, source| {
            b.iter(|| build::typecheck(source, &format!("{}.sop", name)));
        });
    }
    group.finish();
}

fn bench_compile(c: &mut Criterion) {
    let fixtures = load_fixtures();

    let mut group = c.benchmark_group("compile");
    for (name, source) in &fixtures {
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(BenchmarkId::new("file", name), source, |b, source| {
            b.iter(|| build::compile(source, &format!("{}.sop", name)));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_parse, bench_typecheck, bench_compile);
criterion_main!(benches);
