//! Trains epoch on a real file, builds a measure pool (token frequency,
//! context diversity from PPMI), and checks whether axaxes would accept
//! them as axes — balance, independence, and the resulting population
//! split into four cells. This is the real-data check that
//! elements/axes/passport.yaml's open_questions calls for.

use axaxes::{select_pair, Binarizer, Candidate, InteractionMatrix, Quadrant, SelectionCriteria};
use axmva_epoch::EpochState;
use axmva_ppmi::{filter_by_min_level, valid_links, CooAccumulator, ContextExtractor, Window};
use std::collections::HashMap;
use std::env;
use std::fs;

const BIRTH_THRESHOLD: u64 = 2;

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "input/text/three_kingdoms_zh.txt".to_string());
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("не удалось прочитать {path}: {e}"));

    println!("=== axaxes: пул мер на реальном файле ===");
    let mut epoch = EpochState::new();
    epoch.run_corpus(&bytes, BIRTH_THRESHOLD);
    println!("Файл: {path} ({} байт), полка: {}\n", bytes.len(), epoch.store.len());

    let events = Window::range(2, 5).extract(&epoch.store, &epoch.sequence.ids);
    let filtered = filter_by_min_level(&epoch.store, events, 1);
    let mut coo = CooAccumulator::new();
    coo.accumulate(&filtered);
    let csr = coo.bake_to_csr();

    let ids: Vec<u32> = csr.row_index.clone();
    println!("Токенов с хотя бы одной связью (level>=1): {}\n", ids.len());

    let frequency: HashMap<u32, f64> = ids.iter().map(|&id| (id, epoch.store.count_of(id) as f64)).collect();
    let context_diversity: HashMap<u32, f64> = ids.iter().map(|&id| (id, csr.row(id).count() as f64)).collect();

    let pool = vec![
        Candidate { name: "frequency", values: &frequency },
        Candidate { name: "context_diversity", values: &context_diversity },
    ];

    // Диагностика напрямую, не только через select_pair — чтобы увидеть
    // фактические числа даже если порог не пройден (как с контролем
    // перемешиванием в ppmi_demo).
    let freq_values: Vec<f64> = ids.iter().map(|id| frequency[id]).collect();
    let div_values: Vec<f64> = ids.iter().map(|id| context_diversity[id]).collect();
    let bin_freq = Binarizer::from_median(&freq_values);
    let bin_div = Binarizer::from_median(&div_values);
    let balance_freq = bin_freq.balance_fraction(&freq_values);
    let balance_div = bin_div.balance_fraction(&div_values);
    let bits_freq: Vec<bool> = freq_values.iter().map(|&v| bin_freq.classify(v)).collect();
    let bits_div: Vec<bool> = div_values.iter().map(|&v| bin_div.classify(v)).collect();
    let mi = axaxes::mutual_information(&bits_freq, &bits_div);

    println!("Баланс frequency (доля выше медианы): {balance_freq:.3}");
    println!("Баланс context_diversity (доля выше медианы): {balance_div:.3}");
    println!("Взаимная информация между осями: {mi:.4} бит\n");

    let criteria = SelectionCriteria::default();
    match select_pair(&pool, &ids, &criteria) {
        Some(selected) => println!(
            "select_pair: ПРИНЯТО ({} + {}), MI={:.4}\n",
            selected.first_name, selected.second_name, selected.mutual_information
        ),
        None => println!(
            "select_pair: ОТКАЗ — ни одна пара не прошла критерии (max_balance_deviation={}, max_mutual_information={})\n",
            criteria.max_balance_deviation, criteria.max_mutual_information
        ),
    }

    let mut counts = [0usize; 4];
    for (&f, &d) in bits_freq.iter().zip(bits_div.iter()) {
        counts[Quadrant::classify(f, d).as_index()] += 1;
    }
    let total = ids.len() as f64;
    println!("Распределение по 4 клеткам (frequency × context_diversity):");
    for (i, &c) in counts.iter().enumerate() {
        let q = Quadrant::from_index(i);
        println!(
            "  ({:>5}, {:>5}): {:>6} ({:>5.1}%)",
            q.first,
            q.second,
            c,
            100.0 * c as f64 / total
        );
    }

    println!("\nПримеры по клеткам:");
    for target_index in 0..4 {
        let target = Quadrant::from_index(target_index);
        if let Some(pos) = ids.iter().zip(bits_freq.iter().zip(bits_div.iter())).position(|(_, (&f, &d))| Quadrant::classify(f, d) == target) {
            let id = ids[pos];
            let text = String::from_utf8_lossy(&epoch.store.bytes_of(id)).chars().take(12).collect::<String>();
            println!("  ({:>5}, {:>5}) -> {text:?} (freq={}, ctx_div={})", target.first, target.second, frequency[&id], context_diversity[&id]);
        }
    }

    // --- Тузы -> придворные: окружение = сильнейший PPMI-сосед ---
    // Честная оговорка: эта пара мер (frequency × context_diversity) не
    // прошла select_pair выше — это демонстрация механизма InteractionMatrix
    // на реальных парах (своя клетка, клетка окружения), не заявка на то,
    // что это настоящая принятая стихия.
    println!("\n=== Придворные: окружение = сильнейший PPMI-сосед ===");

    let own_cell: HashMap<u32, Quadrant> =
        ids.iter().zip(bits_freq.iter().zip(bits_div.iter())).map(|(&id, (&f, &d))| (id, Quadrant::classify(f, d))).collect();

    let valid = valid_links(&csr, 3, 2.0);
    let mut best_neighbor: HashMap<u32, (u32, f64)> = HashMap::new();
    for entry in &valid {
        best_neighbor
            .entry(entry.token)
            .and_modify(|cur| if entry.ppmi > cur.1 { *cur = (entry.context, entry.ppmi); })
            .or_insert((entry.context, entry.ppmi));
    }

    let mut matrix = InteractionMatrix::new();
    let mut paired = 0usize;
    for (&id, &(neighbor, _ppmi)) in &best_neighbor {
        if let (Some(&own), Some(&surrounding)) = (own_cell.get(&id), own_cell.get(&neighbor)) {
            matrix.record(own, surrounding);
            paired += 1;
        }
    }
    println!(
        "Токенов с сильнейшим соседом: {} | из них у соседа тоже есть своя клетка (учтено в матрице): {}\n",
        best_neighbor.len(),
        paired
    );

    println!("Матрица 4×4 (своя клетка × клетка окружения), доли:");
    println!("                    окружение=(F,F)  окружение=(F,T)  окружение=(T,F)  окружение=(T,T)");
    for own_index in 0..4 {
        let own = Quadrant::from_index(own_index);
        print!("своя=({:>5},{:>5})   ", own.first, own.second);
        for surrounding_index in 0..4 {
            let surrounding = Quadrant::from_index(surrounding_index);
            print!("{:>14.4}   ", matrix.share(own, surrounding));
        }
        println!();
    }
    println!("\nВсего пар учтено в матрице: {}", matrix.total());
}
