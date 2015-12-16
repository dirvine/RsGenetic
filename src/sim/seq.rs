use pheno::Phenotype;
use std::cmp::Ordering;
use rand::Rng;
use rand::distributions::{IndependentSample, Range};
use super::*;
use super::shared::*;
use time::SteadyTime;

/// A `Simulator` can run genetic algorithm simulations in a single thread.
pub struct Simulator<T: Phenotype> {
    population: Vec<Box<T>>,
    max_iters: i32,
    n_iters: i32,
    selection_type: SelectionType,
    fitness_type: FitnessType,
    earlystop: bool,
    earlystopper: EarlyStopper,
    duration: Option<NanoSecond>
}

/// Used for early stopping.
struct EarlyStopper {
    /// Minimum difference required for early stopping.
    delta: f64,
    /// Previously recorded fitness value.
    previous: f64,
    /// Maximum number of iterations before early stopping.
    max_iters: u32,
    /// Current number of iterations where the difference was smaller than delta.
    n_iters: u32,
}

impl EarlyStopper {
    /// Create a new `EarlyStopper`.
    fn new(delta: f64, n_iters: u32) -> EarlyStopper {
        EarlyStopper {
            delta: delta,
            previous: 0.0,
            max_iters: n_iters,
            n_iters: 0,
        }
    }

    /// Update the `EarlyStopper` with a new fitness value.
    ///
    /// Returns whether or not the `Simulator` should stop.
    fn update(&mut self, fitness: f64) -> bool {
        if (fitness - self.previous).abs() < self.delta {
            self.previous = fitness;
            self.n_iters += 1;
        } else {
            self.n_iters = 0;
        }

        self.n_iters >= self.max_iters
    }
}

impl<T: Phenotype> Simulator<T> {
    /// Get the best performing organism.
    fn get(&self) -> Box<T> {
        let mut cloned = self.population.clone();
        cloned.sort_by(|x, y| {
            (*x).fitness().partial_cmp(&(*y).fitness()).unwrap_or(Ordering::Equal)
        });
        match self.fitness_type {
            FitnessType::Maximize => cloned[cloned.len() - 1].clone(),
            FitnessType::Minimize => cloned[0].clone(),
        }
    }
}

impl<T: Phenotype> Simulation<T, SimulatorBuilder<T>> for Simulator<T> {
    /// Create builder.
    fn builder(pop: Vec<Box<T>>) -> SimulatorBuilder<T> {
        SimulatorBuilder {
            sim: Simulator {
                population: pop,
                max_iters: 100,
                n_iters: 0,
                selection_type: SelectionType::Maximize { count: 5 },
                fitness_type: FitnessType::Maximize,
                earlystop: false,
                earlystopper: EarlyStopper::new(0.0, 0),
                duration: None
            },
        }
    }

    /// Run.
    fn run(&mut self) -> Result<Box<T>, String> {
        let time_start = SteadyTime::now();
        while self.n_iters < self.max_iters {
            // Perform selection
            let parents_tmp = match self.selection_type {
                SelectionType::Maximize{count: c} => self.selection_maximize(c),
                SelectionType::Tournament{num: n, count: c} => self.selection_tournament(n, c),
                SelectionType::Stochastic{count: c} => self.selection_stochastic(c),
                SelectionType::Roulette{count: c} => self.selection_roulette(c),
            };
            if parents_tmp.is_err() {
                return Err(parents_tmp.err().unwrap());
            }
            let parents = parents_tmp.ok().unwrap();
            // Create children from the selected parents and mutate them
            let children: Vec<Box<T>> = parents.iter()
                                               .map(|pair: &(Box<T>, Box<T>)| {
                                                   pair.0.crossover(&*(pair.1))
                                               })
                                               .map(|c| Box::new(c.mutate()))
                                               .collect();
            // Kill off parts of the population at random to make room for the children
            match self.kill_off(children.len()) {
                Ok(_) => {
                    for child in children {
                        self.population.push(child);
                    }

                    self.n_iters += 1;
                }
                Err(e) => {
                    return Err(e);
                }
            }

            if self.earlystop {
                let mut cloned = self.population.clone();
                cloned.sort_by(|x, y| {
                    (*x).fitness().partial_cmp(&(*y).fitness()).unwrap_or(Ordering::Equal)
                });
                let highest_fitness = match self.fitness_type {
                                          FitnessType::Maximize => cloned[cloned.len() - 1].clone(),
                                          FitnessType::Minimize => cloned[0].clone(),
                                      }
                                      .fitness();

                if self.earlystopper.update(highest_fitness) {
                    break;
                }
            }
        }
        self.duration = (SteadyTime::now() - time_start).num_nanoseconds();
        Ok(self.get())
    }

    fn iterations(&self) -> i32 {
        self.n_iters
    }

    fn time(&self) -> Option<NanoSecond> {
        self.duration
    }
}

impl<T: Phenotype> Selector<T> for Simulator<T> {
    /// Select count*2 parents for breeding.
    fn selection_maximize(&self, count: u32) -> Result<Parents<T>, String> {
        if count <= 0 || ((count * 2) as usize) >= self.population.len() {
            return Err(format!("Invalid parameter `count`: {}. Should be larger than zero and \
                                less than half the population size.",
                               count));
        }

        let mut cloned = self.population.clone();
        cloned.sort_by(|x, y| {
            (*x).fitness().partial_cmp(&(*y).fitness()).unwrap_or(Ordering::Equal)
        });
        if let FitnessType::Maximize = self.fitness_type {
            cloned.reverse();
        }
        let sorted: Vec<&Box<T>> = cloned.iter().take(2 * (count as usize)).collect();
        let mut index = 0;
        let mut result: Parents<T> = Vec::new();
        while index < sorted.len() {
            result.push((sorted[index].clone(), sorted[index + 1].clone()));
            index += 2;
        }
        Ok(result)
    }

    /// Select parents using tournament selection.
    fn selection_tournament(&self, num: u32, count: u32) -> Result<Parents<T>, String> {
        if num <= 0 || ((num * 2) as usize) >= self.population.len() {
            return Err(format!("Invalid parameter `num`: {}. Should be larger than zero and \
                                less than half the population size.",
                               num));
        }
        if count <= 0 || (count as usize) >= self.population.len() {
            return Err(format!("Invalid parameter `count`: {}. Should be larger than zero and \
                                less than half the population size.",
                               count));
        }

        let mut result: Parents<T> = Vec::new();
        let mut rng = ::rand::thread_rng();
        for _ in 0..num {
            let mut tournament: Vec<Box<T>> = Vec::with_capacity(count as usize);
            for _ in 0..count {
                let index = rng.gen::<usize>() % self.population.len();
                tournament.push(self.population[index].clone());
            }
            tournament.sort_by(|x, y| {
                (*x).fitness().partial_cmp(&(*y).fitness()).unwrap_or(Ordering::Equal)
            });
            match self.fitness_type {
                FitnessType::Maximize => {
                    result.push((tournament[tournament.len() - 1].clone(),
                                 tournament[tournament.len() - 2].clone()));
                }
                FitnessType::Minimize => {
                    result.push((tournament[0].clone(), tournament[1].clone()));
                }
            }
        }
        Ok(result)
    }

    /// Select parents using stochastic universal sampling.
    fn selection_stochastic(&self, count: u32) -> Result<Parents<T>, String> {
        if count <= 0 || (count as usize) >= self.population.len() {
            return Err(format!("Invalid parameter `count`: {}. Should be larger than zero and \
                                less than the population size.",
                               count));
        }

        let ratio = self.population.len() / (count as usize);
        let mut result: Parents<T> = Vec::new();
        let mut i = ::rand::random::<usize>() % self.population.len() as usize;
        let mut selected = 0;
        while selected < count {
            result.push((self.population[i].clone(),
                         self.population[(i + ratio - 1) % self.population.len()].clone()));
            i += ratio - 1;
            i = i % self.population.len();
            selected += 2;
        }
        Ok(result)
    }

    /// Select parents using roulette wheel selection.
    fn selection_roulette(&self, count: u32) -> Result<Parents<T>, String> {
        if count <= 0 || (count as usize) >= self.population.len() {
            return Err(format!("Invalid parameter `count`: {}. Should be larger than zero and \
                                less than the population size.",
                               count));
        }

        let mut results: Parents<T> = Vec::new();

        let mut cloned = self.population.clone();
        cloned.sort_by(|x, y| {
            (*x).fitness().partial_cmp(&(*y).fitness()).unwrap_or(Ordering::Equal)
        });
        let max_fitness = cloned[cloned.len() - 1].fitness();

        let between = Range::new(0.0, 1.0);
        let mut rng = ::rand::thread_rng();

        let mut selected = 0;
        while selected < count {
            let mut inner_selected: Vec<Box<T>> = Vec::with_capacity(2);
            while inner_selected.len() < 2 {
                let i = rng.gen::<usize>() % self.population.len() as usize;
                let c = between.ind_sample(&mut rng);

                let frac = self.population[i].fitness() / max_fitness;
                if c <= frac {
                    inner_selected.push(self.population[i].clone());
                }
            }
            results.push((inner_selected[0].clone(), inner_selected[1].clone()));

            selected += 2;
        }
        Ok(results)
    }

    /// Kill off phenotypes using stochastic universal sampling.
    fn kill_off(&mut self, count: usize) -> Result<(), String> {
        let old_len = self.population.len();
        let ratio = self.population.len() / count;
        let mut i = ::rand::random::<usize>() % self.population.len() as usize;
        let mut selected = 0;
        while selected < count {
            self.population.remove(i);
            i += ratio - 1;
            i = i % self.population.len();

            selected += 1;
        }
        if self.population.len() == old_len - count {
            Ok(())
        } else {
            Err(format!("Something went wrong during reduction of population. Invalid number of \
                         results."))
        }
    }
}

/// A `Builder` for the `Simulator` type.
pub struct SimulatorBuilder<T: Phenotype> {
    sim: Simulator<T>,
}

impl<T: Phenotype> SimulatorBuilder<T> {
    /// Set the maximum number of iterations of the resulting `Simulator`.
    ///
    /// The `Simulator` will stop running after this number of iterations.
    ///
    /// Returns itself for chaining purposes.
    pub fn set_max_iters(mut self, i: i32) -> Self {
        self.sim.max_iters = i;
        self
    }

    /// Set the selection type of the resulting `Simulator`.
    ///
    /// Returns itself for chaining purposes.
    pub fn set_selection_type(mut self, t: SelectionType) -> Self {
        self.sim.selection_type = t;
        self
    }

    /// Set the fitness type of the resulting `Simulator`,
    /// determining whether the `Simulator` will try to maximize
    /// or minimize the fitness values of `Phenotype`s.
    ///
    /// Returns itself for chaining purposes.
    pub fn set_fitness_type(mut self, t: FitnessType) -> Self {
        self.sim.fitness_type = t;
        self
    }

    /// Set early stopping. If for `n_iters` iterations, the change in the highest fitness
    /// is smaller than `delta`, the simulator will stop running.
    ///
    /// Returns itself for chaining purposes.
    pub fn set_early_stop(mut self, delta: f64, n_iters: u32) -> Self {
        self.sim.earlystop = true;
        self.sim.earlystopper = EarlyStopper::new(delta, n_iters);
        self
    }
}

impl<T: Phenotype> Builder<Box<Simulator<T>>> for SimulatorBuilder<T> {
    fn build(self) -> Box<Simulator<T>> {
        Box::new(self.sim)
    }
}

#[cfg(test)]
mod tests {
    use super::*; // seq
    use super::super::*; // sim
    use pheno;
    use std::cmp;

    struct Test {
        i: i32,
    }

    impl pheno::Phenotype for Test {
        fn fitness(&self) -> f64 {
            (self.i - 0).abs() as f64
        }

        fn crossover(&self, t: &Test) -> Test {
            Test { i: cmp::min(self.i, t.i) }
        }

        fn mutate(&self) -> Test {
            if self.i < 0 {
                Test { i: self.i + 1 }
            } else {
                Test { i: self.i - 1 }
            }
        }
    }

    impl Clone for Test {
        fn clone(&self) -> Self {
            Test { i: self.i }
        }
    }

    #[test]
    #[should_panic]
    fn test_maximize_count_0() {
        let tests = (0..100).map(|i| Box::new(Test { i: i })).collect();
        let mut s = *seq::Simulator::builder(tests)
                         .set_max_iters(100)
                         .set_selection_type(SelectionType::Maximize { count: 0 })
                         .set_fitness_type(FitnessType::Minimize)
                         .build();
        s.run().unwrap();
    }

    #[test]
    #[should_panic]
    fn test_tournament_count_0() {
        let tests = (0..100).map(|i| Box::new(Test { i: i })).collect();
        let mut s = *seq::Simulator::builder(tests)
                         .set_max_iters(100)
                         .set_selection_type(SelectionType::Tournament { num: 2, count: 0 })
                         .set_fitness_type(FitnessType::Minimize)
                         .build();
        s.run().unwrap();
    }

    #[test]
    #[should_panic]
    fn test_tournament_num_0() {
        let tests = (0..100).map(|i| Box::new(Test { i: i })).collect();
        let mut s = *seq::Simulator::builder(tests)
                         .set_max_iters(100)
                         .set_selection_type(SelectionType::Tournament { num: 0, count: 1 })
                         .set_fitness_type(FitnessType::Minimize)
                         .build();
        s.run().unwrap();
    }

    #[test]
    #[should_panic]
    fn test_stochastic_count_0() {
        let tests = (0..100).map(|i| Box::new(Test { i: i })).collect();
        let mut s = *seq::Simulator::builder(tests)
                         .set_max_iters(100)
                         .set_selection_type(SelectionType::Stochastic { count: 0 })
                         .set_fitness_type(FitnessType::Minimize)
                         .build();
        s.run().unwrap();
    }

    #[test]
    fn test_runtime() {
        let tests = (0..100).map(|i| Box::new(Test { i: i })).collect();
        let mut s = *seq::Simulator::builder(tests)
                         .set_max_iters(2000)
                         .set_selection_type(SelectionType::Stochastic { count: 1 })
                         .set_fitness_type(FitnessType::Minimize)
                         .build();
        s.run().unwrap();
        s.time().unwrap();
    }

    #[test]
    #[should_panic]
    fn test_time_norun() {
        let tests = (0..100).map(|i| Box::new(Test { i: i })).collect();
        let s = *seq::Simulator::builder(tests)
                         .build();
        s.time().unwrap();
    }

    #[test]
    fn test_earlystop() {
        let tests = (0..100).map(|i| Box::new(Test { i: i })).collect();
        let mut s_early = *seq::Simulator::builder(tests)
                               .set_max_iters(1000)
                               .set_selection_type(SelectionType::Stochastic { count: 5 })
                               .set_fitness_type(FitnessType::Minimize)
                               .set_early_stop(0.1, 5)
                               .build();
        let tests2 = (0..100).map(|i| Box::new(Test { i: i })).collect();
        let mut s_no_early = *seq::Simulator::builder(tests2)
                                  .set_max_iters(1000)
                                  .set_selection_type(SelectionType::Stochastic { count: 5 })
                                  .set_fitness_type(FitnessType::Minimize)
                                  .build();
        s_early.run().unwrap(); // Both should run without error.
        s_no_early.run().unwrap(); // Both should run without error.

        assert!(s_early.iterations() <= s_no_early.iterations());
    }

    #[test]
    fn simple_convergence_test_maximize() {
        let tests = (0..100).map(|i| Box::new(Test { i: i + 10 })).collect();
        let mut s = *seq::Simulator::builder(tests)
                         .set_max_iters(1000)
                         .set_selection_type(SelectionType::Maximize { count: 5 })
                         .set_fitness_type(FitnessType::Minimize)
                         .build();
        s.run().unwrap();
        assert_eq!((*s.get()).i, 0);
    }

    #[test]
    fn simple_convergence_test_tournament() {
        let tests = (0..100).map(|i| Box::new(Test { i: i + 10 })).collect();
        let mut s = *seq::Simulator::builder(tests)
                         .set_max_iters(1000)
                         .set_selection_type(SelectionType::Tournament { count: 3, num: 5 })
                         .set_fitness_type(FitnessType::Minimize)
                         .build();
        s.run().unwrap();
        assert_eq!((*s.get()).i, 0);
    }

    #[test]
    fn simple_convergence_test_stochastic() {
        let tests = (0..100).map(|i| Box::new(Test { i: i + 10 })).collect();
        let mut s = *seq::Simulator::builder(tests)
                         .set_max_iters(1000)
                         .set_selection_type(SelectionType::Stochastic { count: 5 })
                         .set_fitness_type(FitnessType::Minimize)
                         .build();
        s.run().unwrap();
        assert_eq!((*s.get()).i, 0);
    }

    #[test]
    fn simple_convergence_test_roulette() {
        let tests = (0..100).map(|i| Box::new(Test { i: i + 10 })).collect();
        let mut s = *seq::Simulator::builder(tests)
                         .set_max_iters(1000)
                         .set_selection_type(SelectionType::Roulette { count: 4 })
                         .set_fitness_type(FitnessType::Minimize)
                         .build();
        s.run().unwrap();
        assert!((*s.get()).i == 0);
    }
}
