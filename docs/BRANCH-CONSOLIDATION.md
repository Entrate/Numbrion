# Branch consolidation - 2026-10-10

`main` is the development branch. It contains the completed `engine` implementation,
Windows DirectML training, and the history of every former local and remote branch.

Branch histories are merged without rebasing or force-pushing `main`. Older work-in-progress
snapshots were reviewed against their changed files, the current implementation, and the
existing recovery/porting reports. Most work had already been adopted in later commits.
Those snapshots are joined with the `ours` merge strategy to preserve their original commits
while retaining the completed implementation. The unrelated local template `master` is
also preserved: its icon is already present and its README is superseded by the project README.

The unmerged outside move-execution test and generator are recovered into the active tree.
Unfinished extra stats-oracle rows on `finish/core-queries` and the older isolated healing
callback fixtures remain available through their original commits below; they are not enabled
as current tests. The reactive-stats recording-host helpers were deliberately replaced by
direct core API ports, as documented in `design/reports/effects-reactive_stats.md`.

Use `git show <commit>:<path>` to retrieve any historical file after branch deletion.
A complete local Git bundle and original ref manifest are saved under ignored `scratch/`:
`pre-main-consolidation.bundle` and `pre-main-consolidation-refs.txt`.

| Former branch | Original head | Disposition |
| --- | --- | --- |
| `engine` | `9473b8ea6e4f4da83ee02c78f279ed1cc9949f85` | Completed engine and Windows training merged into main. |
| `finish/core-queries` | `32c2aacfbfb980513f16c8157db9562206b5028f` | Core queries already implemented; extra unfinished stats probe retained in history. |
| `finish/items-modifiers` | `a4c1ab087f1c91b9dfd441db78e8aa28d4d18b14` | Already an ancestor of engine. |
| `finish/passive-offense` | `bb5ba4094ce6bffa0a70b316aeb7a3d72164b661` | Already an ancestor of engine. |
| `finish/reactive-contact` | `a874a2d1385fe3757ef5a4f2dec7d2ace1fa7cb0` | Already an ancestor of engine. |
| `finish/reactive-stats` | `45cbb1acb3967199e742e57fd5b281f7495b1bc3` | Core query commits adopted; effects rewritten against current APIs (effects-reactive_stats.md). |
| `main` | `5d3f0da7fcfb373c9ebc005fd9e0f86e6caefa12` | Merge completed engine into existing main. |
| `recovery/stage2b-2026-10-09` | `f902b5b90a6713e95029bf156b1f29e4e636915e` | Already an ancestor of engine. |
| `salvage/choices-legal-joint-undo` | `441ad3b005967a2e7af4f45e71c9fa59d6f9eabd` | Choice legality/undo fixes adopted and subsequently updated. |
| `wip/agent-a0ca720a924313ab2` | `15de6f6bd179e1161d3097ac78d8aeb66503e533` | Log replay completed in c90c50f and later fixes. |
| `wip/agent-a0e74313084948dba` | `3ed007c5abac05beb52bd4142cf5a41dcf591dc5` | Earlier integration snapshot; keep current optional tracing and difftest configuration. |
| `wip/agent-a12f92ee457928654` | `7b4fff9190365d4e61b0ef927183c036165c285a` | Completed disable/lock/trap port and scenarios retained. |
| `wip/agent-a24d96f62beb495ed` | `bee3d1d6f2bb18e75365cdd838d495aaf8f7cbef` | Completed rules/status port and scenarios retained. |
| `wip/agent-a384c9c746b1889f6` | `73dbbbac19cb9805b9f9698a4a50e2b176dc9d32` | Completed charge/priority/multihit port and corrected source references retained. |
| `wip/agent-a449c98c04e2cc31f` | `c5e37e007c4e26d3ffaef4df7148b7b87f085acd` | Choice work adopted; keep current test fixtures and lifecycle integration. |
| `wip/agent-a4846c27b9804375e` | `7e8dd3b9e1709005cebd4684230c7b5968e87679` | Hazards/screens implementation adopted; keep later tests and source references. |
| `wip/agent-a4a6a456dc934f403` | `edbd2d163c03691a88321fc9ee502cccc87e4d98` | Lifecycle oracle tests adopted and enabled with scoped request recording. |
| `wip/agent-a4b279b145a3d48f7` | `676eb5ee348dd30a0f49953e9fbd92cbf79ba126` | Natural Cure and event collection adopted with later fixes/optimizations. |
| `wip/agent-aa8d861be86a3f31d` | `936936c8f301641ad00052e1c873f1b419c2c13b` | Recover outside move-execution test, its 42 oracle vectors, and generator. |
| `wip/agent-aad5404d57a6ed1b3` | `fcd1c2f92566da99428c703ac3ea18d73babd6db` | Consumables helper reviewed and completed (effects-consumables.md). |
| `wip/agent-ad00da993a3fec476` | `883fe20eae3bc2804c611b90a317002bfcfba874` | Healing/residual fixes adopted; keep completed report. |
| `wip/agent-ad6a17e0b788368ed` | `0306e4bd44a997de74ed4f85fe9c6dfa7606577f` | Python benchmark identical; keep completed training API and Windows additions. |
| `wip/agent-aedeb90cfb8820d7f` | `7692e33843ad8ab74de8e44557ca16f3459398e9` | Weather/terrain scenarios adopted and corrected for legal choices after faints. |
| `wip/agent-af2654e454fa938c7` | `c6c55439fc1d422ec2a0295fd42847bec09bd03e` | Healing/residual effects adopted with later fixes and scenario coverage. |
| `wip/gpt-formes` | `7473a75b4ae6fb589a19f9837a00286077b192ab` | Formes/Tera/identity effects adopted with later fixes and broader oracle tests. |
| `wip/gpt-mutators` | `6b1c1fcde87019c08377db78b38c6f89bf9d9b06` | Mutators/damage work adopted with later parity corrections. |
| `wip/gpt-perf1` | `5ec537a453abca56559b584aba86ba577aaedbe1` | Hook index adopted and extended by the completed performance round. |
| `wip/main-checkout-0919` | `d4e32e3ddaf2b54951ecf79abdf9c26aa883c41f` | Effects adopted; old direct callback vectors retained in history, current full-battle tests retained. |
| Local `master` | `032c8be139803ee3821857454f0891432953e30c` | Template history preserved; current icon and README retained. |
