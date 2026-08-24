use std::{hint::black_box, sync::Arc, time::Instant};

use org_studio::{
    command::{
        ArgumentSpec, Availability, BuiltinCommand, BuiltinCommandSpec, CapabilitySet,
        CommandRegistryBuilder, CommandRole, RedactionPolicy, RepeatPolicy, SideEffectClass,
        UndoPolicy,
    },
    keymap::{ActiveKeymaps, KeyLookup, KeySequence, KeymapBuilder, StrokeInterner},
};

const BINDING_COUNT: usize = 4_096;
const LOOKUP_COUNT: usize = 2_000_000;

fn main() {
    let command = benchmark_command();
    let mut interner = StrokeInterner::default();
    let mut builder = KeymapBuilder::new();
    let mut sequences = Vec::with_capacity(BINDING_COUNT);

    let build_started = Instant::now();
    for index in 0..BINDING_COUNT {
        let source = format!("C-x benchmark-{index}");
        let sequence = KeySequence::parse(&source).expect("generated sequence is valid");
        let sequence = interner.intern_sequence(&sequence);
        builder
            .bind(&sequence, command)
            .expect("generated bindings are unique");
        sequences.push(sequence);
    }
    let base = Arc::new(builder.freeze(1));
    let build_elapsed = build_started.elapsed();

    let direct = time_lookups(LOOKUP_COUNT, |index| {
        base.lookup(&sequences[index % sequences.len()])
    });

    let overlays = (0..3)
        .map(|generation| Arc::new(KeymapBuilder::new().freeze(generation + 2)))
        .collect();
    let active = ActiveKeymaps::new(4, None, overlays, base)
        .expect("three minor overlays are within the hard limit");
    let layered = time_lookups(LOOKUP_COUNT, |index| {
        active.resolve(&sequences[index % sequences.len()])
    });

    println!("keymap_benchmark bindings={BINDING_COUNT} lookups={LOOKUP_COUNT}");
    println!(
        "compile_ms={:.3} direct_ns_per_lookup={:.2} layered_ns_per_lookup={:.2}",
        build_elapsed.as_secs_f64() * 1_000.0,
        nanos_per_lookup(direct, LOOKUP_COUNT),
        nanos_per_lookup(layered, LOOKUP_COUNT),
    );
}

fn time_lookups(mut count: usize, mut lookup: impl FnMut(usize) -> KeyLookup) -> std::time::Duration {
    let total = count;
    let started = Instant::now();
    while count > 0 {
        let result = lookup(total - count);
        assert!(matches!(black_box(result), KeyLookup::Command(_)));
        count -= 1;
    }
    started.elapsed()
}

fn nanos_per_lookup(elapsed: std::time::Duration, count: usize) -> f64 {
    elapsed.as_secs_f64() * 1_000_000_000.0 / count as f64
}

fn benchmark_command() -> org_studio::command::CommandKey {
    let mut registry = CommandRegistryBuilder::default();
    registry
        .register_builtin(BuiltinCommandSpec {
            name: "org-studio.benchmark.noop".into(),
            aliases: &[],
            title: "Benchmark no-op",
            description: "Synthetic command used by the keymap benchmark",
            command: BuiltinCommand::ReloadDocument,
            role: CommandRole::Action,
            argument_spec: ArgumentSpec::None,
            repeat: RepeatPolicy::Never,
            undo: UndoPolicy::None,
            availability: Availability::Always,
            side_effect: SideEffectClass::None,
            required_capabilities: CapabilitySet::empty(),
            redaction: RedactionPolicy::DoNotRecord,
        })
        .expect("benchmark command descriptor is valid")
}
