use criterion::{criterion_group, criterion_main, Criterion};

pub fn benchmark(c: &mut Criterion) {
    c.benchmark_group("eval(return hello)")
        .bench_function("remolt", |b| {
            let mut tcl = remolt::Interp::new();
            tcl.eval("proc benchy {} {return hello}").unwrap();
            b.iter(move || {
                tcl.eval("benchy").unwrap();
            })
        });
    c.benchmark_group("nested-ifs")
        .bench_function("remolt", |b| {
            let mut tcl = remolt::Interp::new();
            tcl.eval("proc benchy {} {if {0 == 0} {if {0 == 0} {if {0 == 0} {}}}}").unwrap();
            b.iter(move || {
                tcl.eval("benchy").unwrap();
            })
        });
    c.benchmark_group("complex-expr")
        .bench_function("remolt", |b| {
            let mut tcl = remolt::Interp::new();
            tcl.eval("proc benchy {} {set a 5; set b 7; expr {($a + $b) * 4 - 0}}").unwrap();
            b.iter(move || {
                tcl.eval("benchy").unwrap();
            })
        });
    c.benchmark_group("call-proc")
        .bench_function("remolt", |b| {
            let mut tcl = remolt::Interp::new();
            tcl.eval( "proc testproc {x y z} { }").unwrap();
            tcl.eval( "proc benchy {} { testproc a b c }").unwrap();
            b.iter(move || {
                tcl.eval("benchy").unwrap();
            })
        });
    c.benchmark_group("recursive-fib-5")
        .bench_function("remolt", |b| {
            let mut tcl = remolt::Interp::new();
            tcl.eval("\
                proc fib {x} { \
                    if {$x <= 1} {return 1} else { \
                        return [expr {[fib [expr {$x - 1}]] + [fib [expr {$x - 2}]]}] \
                    } \
                }").unwrap();
            b.iter(move || {
                tcl.eval("fib 5").unwrap();
            })
        });
    // Don't really have any benchmarks testing variable binding and access, so
    // here we go.
    //
    // 45 is the largest fibonacci sequence number that can be represented in
    // our i32 integer type.
    c.benchmark_group("iterative-fib-45")
        .bench_function("remolt", |b| {
            let mut tcl = remolt::Interp::new();
            tcl.eval("\
                proc fib {x} { \
                    set a 0; \
                    set b 1; \
                    while {$x != 0} { \
                        set x [expr {$x - 1}] ; \
                        set t $b; \
                        set b [expr {$a + $b}]; \
                        set a $t \
                    }; return $a \
                }").unwrap();
            b.iter(move || {
                tcl.eval("fib 45").unwrap();
            })
        });
}

criterion_group!(benches, benchmark);
criterion_main!(benches);
