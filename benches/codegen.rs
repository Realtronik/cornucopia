use cornucopia::{conn::cornucopia_conn, CodegenSettings};
use criterion::Criterion;

fn bench(c: &mut Criterion) {
    cornucopia::container::cleanup(false).ok();
    cornucopia::container::setup(false).unwrap();
    let client = &mut cornucopia_conn().unwrap();

    cornucopia::load_schema(client, &["../codegen_test/schema.sql"]).unwrap();
    c.bench_function("codegen_sync", |b| {
        b.iter(|| {
            cornucopia::generate_live(
                client,
                "../test_codegen/queries",
                None,
                CodegenSettings {
                    gen_sync: true,
                    gen_async: false,
                    gen_use: vec![],
                    gen_derive: vec!["serde::Serialize".to_string()],
                    gen_utoipa_enum_fix: false,
                },
            )
            .unwrap()
        })
    });
    c.bench_function("codegen_async", |b| {
        b.iter(|| {
            cornucopia::generate_live(
                client,
                "../test_codegen/queries",
                None,
                CodegenSettings {
                    gen_sync: true,
                    gen_async: false,
                    gen_use: vec![],
                    gen_derive: vec!["serde::Serialize".to_string()],
                    gen_utoipa_enum_fix: false,
                },
            )
            .unwrap()
        })
    });
    cornucopia::container::cleanup(false).unwrap();
}
criterion::criterion_group!(benches, bench);
criterion::criterion_main!(benches);
