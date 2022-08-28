A fuzzer for finding differences between Ruffle and Flash Player

### How it works
Generates random flash SWFs and runs them through both Ruffle and Flash Player,
comparing the outputs to find discrepancies

In order to run multiple versions of Flash Player Projector at the same time, an LD_PRELOAD harness is used to redirect the flashlog.txt to stdout

Running with a large number of threads can cause issues, such as crashing desktop sessions (Observed on KDE, probably due to GPU drivers), to work around this, or to run on headless systems this can also be ran in a docker container

### Running
- Make sure the latest version of ruffle with the required patches is referenced in cargo.toml
- Customise the config in main.rs
#### Locally
`cargo run --release`

#### Docker

Either
- `docker-compose up --build`

or
- `./deploy-multithread.sh` 
