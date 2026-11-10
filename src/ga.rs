//! A small elitist evolution strategy.
//!
//! Numeric genes move by gaussian mutation and blend crossover. The law
//! changes only when the language-model backend says so. With the mock
//! backend that proposal is a fixed schedule, so the search improves on
//! the demo for every seed the tests cover.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::fitness::{self, Fitness};
use crate::genome::{Genes, Genome, LawKind};
use crate::llm::{apply_proposal, LlmBackend, MutationRequest};
use crate::rng::Pcg64;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenerationRecord {
    pub generation: u32,
    pub best_fitness: f64,
    pub best_law: LawKind,
    pub mean_fitness: f64,
    pub note: String,
}

#[derive(Clone, Debug)]
pub struct Individual {
    pub genome: Genome,
    pub fitness: Fitness,
}

#[derive(Clone, Debug)]
pub struct Evolution {
    pub best: Individual,
    pub archive: Vec<Individual>,
    pub history: Vec<GenerationRecord>,
    pub evaluations: usize,
}

pub struct Engine {
    pub population: usize,
    pub generations: usize,
    pub seed: u64,
}

impl Default for Engine {
    fn default() -> Self {
        Self {
            population: 16,
            generations: 12,
            seed: 1,
        }
    }
}

impl Engine {
    pub fn run(&self, llm: &dyn LlmBackend) -> Result<Evolution> {
        let population = self.population.max(4);
        let mut rng = Pcg64::new(self.seed, 3);
        let mut people: Vec<Individual> = (0..population)
            .map(|_| {
                let genome = initial_genome(&mut rng);
                let fitness = fitness::evaluate(&genome);
                Individual { genome, fitness }
            })
            .collect();
        let mut history = Vec::new();
        let mut archive = Vec::new();
        let mut evaluations = people.len();

        for generation in 0..self.generations {
            people.sort_by(|a, b| {
                a.fitness
                    .score
                    .partial_cmp(&b.fitness.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for person in &people {
                consider_archive(&mut archive, person);
            }
            let best = &people[0];
            let mean = people.iter().map(|p| p.fitness.score).sum::<f64>() / people.len() as f64;
            let proposal = llm.mutate(&MutationRequest {
                generation: generation as u32,
                law: best.genome.law,
                genes: best.genome.genes.clone(),
                fitness: best.fitness.score,
                source: crate::emit::emit(&best.genome),
                diagnostics: String::new(),
            })?;
            history.push(GenerationRecord {
                generation: generation as u32,
                best_fitness: best.fitness.score,
                best_law: best.genome.law,
                mean_fitness: mean,
                note: proposal.note.clone(),
            });

            if generation + 1 == self.generations {
                break;
            }

            let elites = 2.min(people.len());
            let mut next = Vec::with_capacity(population);
            for elite in people.iter().take(elites) {
                next.push(elite.genome.clone());
            }
            if proposal.law.is_some() {
                let mut law = best.genome.law;
                let mut genes = best.genome.genes.clone();
                apply_proposal(&mut law, &mut genes, &proposal);
                next.push(Genome::new(law, genes));
            }
            while next.len() < population {
                let left = tournament(&people, &mut rng);
                let right = tournament(&people, &mut rng);
                let mut child = if rng.chance(0.7) {
                    left.genome
                        .genes
                        .crossover(&right.genome.genes, rng.uniform(0.35, 0.65))
                } else {
                    left.genome.genes.clone()
                };
                let mut law = left.genome.law;
                let sigma = 0.18 * 0.94_f64.powi(generation as i32);
                child.mutate(&mut rng, sigma);
                if law == best.genome.law {
                    apply_proposal(&mut law, &mut child, &proposal);
                }
                next.push(Genome::new(law, child));
            }
            people = next
                .into_iter()
                .map(|genome| {
                    let fitness = fitness::evaluate(&genome);
                    evaluations += 1;
                    Individual { genome, fitness }
                })
                .collect();
        }

        people.sort_by(|a, b| {
            a.fitness
                .score
                .partial_cmp(&b.fitness.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(Evolution {
            best: people.remove(0),
            archive,
            history,
            evaluations,
        })
    }
}

fn initial_genome(rng: &mut Pcg64) -> Genome {
    let mut genes = Genes::zeros();
    genes.kp = rng.uniform(0.0, 0.03);
    genes.bias = rng.uniform(-0.01, 0.02);
    Genome::new(LawKind::Proportional, genes)
}

fn tournament<'a>(people: &'a [Individual], rng: &mut Pcg64) -> &'a Individual {
    let mut best = &people[rng.gen_index(people.len())];
    for _ in 0..2 {
        let other = &people[rng.gen_index(people.len())];
        if other.fitness.score < best.fitness.score {
            best = other;
        }
    }
    best
}

fn same_objectives(left: &crate::fitness::Fitness, right: &crate::fitness::Fitness) -> bool {
    left.objectives()
        .iter()
        .zip(right.objectives())
        .all(|(a, b)| a.to_bits() == b.to_bits())
}

fn consider_archive(archive: &mut Vec<Individual>, candidate: &Individual) {
    if !candidate.fitness.score.is_finite() {
        return;
    }
    if archive.iter().any(|member| {
        member.fitness.dominates(&candidate.fitness)
            || same_objectives(&member.fitness, &candidate.fitness)
    }) {
        return;
    }
    archive.retain(|member| !candidate.fitness.dominates(&member.fitness));
    archive.push(candidate.clone());
    if archive.len() > 8 {
        archive.sort_by(|a, b| {
            a.fitness
                .score
                .partial_cmp(&b.fitness.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        archive.truncate(8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::MockBackend;

    #[test]
    fn demo_search_improves_and_is_deterministic() {
        let engine = Engine {
            population: 16,
            generations: 12,
            seed: 1,
        };
        let once = engine.run(&MockBackend).unwrap();
        let twice = engine.run(&MockBackend).unwrap();
        assert_eq!(
            once.best.fitness.score.to_bits(),
            twice.best.fitness.score.to_bits()
        );
        let start = once.history[0].best_fitness;
        let end = once.best.fitness.score;
        assert!(
            start > 400.0,
            "initial best should be a weak proportional controller, got {start}"
        );
        assert!(
            end < start * 0.5,
            "fitness did not improve enough: {start} -> {end}"
        );
        assert!(end < 400.0, "final fitness {end} is still weak");
        assert_ne!(once.best.genome.law, LawKind::Proportional);
        assert_ne!(once.best.genome.law, LawKind::Hold);
    }

    #[test]
    fn several_seeds_improve() {
        for seed in [1_u64, 2, 3, 4, 5] {
            let engine = Engine {
                population: 16,
                generations: 12,
                seed,
            };
            let run = engine.run(&MockBackend).unwrap();
            let start = run.history[0].best_fitness;
            let end = run.best.fitness.score;
            assert!(
                end < start * 0.5,
                "seed {seed} did not improve: {start} -> {end}"
            );
        }
    }
}
