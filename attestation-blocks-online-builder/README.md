# Listen to latest source chain blocks and build attestation chain
This program builds attestation chain from source chain blocks being listened to online.

## Assumptions and conditions
- source chain blocks are listened to using a pubsub wss pipeline
- source chain blocks are announced in-order (observed)
- a freshly announced block's transactions are not available just yet on the source chain.
  In order to retrieve the block's data it is necessary to wait for a certain amount of time
  (the block time was chosen here) and use the API calls utilized for historical block data fetching.
If the block data is attempted to be retrieved immediately an error "block is being processed" is obtained.
A possible reason for that is that the pubsub pipeline delivers blocks gossiped by the peers and they have not been yet put on the source chain (this is just my conjecture).
- the block data retrieval tasks deliver data out-of-order
- the client network connection is unreliable
- the wss provider has certain ability of buffering blocks and delivering them later after the broken network connection is recovered (observed)

## Goals of the program
Taking in account the above considerations we want to continuosly build an attestation chain in-order and offer certain resiliency to
(reasonably short) network failures on the client side. How short managable network failures are may depend on the buffering capabilities
of the wss provider, the latency lag between the attestation and source chains we consider acceptable
(until we optimize the attestation chain creation rate) and, maybe, other factors.

## Program structure
### Source chain block listener
This task listens to the subscribed channel for the announced blocks.
As described above it's necessary to store these blocks for until they're eventually available on the source chain.
So the first stage of the block lifecycle is the purgatory queue (see below) where they stay until expulsed.
The second branch of this task awakes at regular timeouts and checks for "blocks" in the purgatory queue that can be expulsed and further consumed.

### Purgatory queue
Contains block numbers and timestamps of the announced blocks.
All the "blocks" that stayed in the purgatory for more time then the purgatory period (source chain block time was tried so far)
are considered to be ready on the source chain and safe for polling.
These "blocks" are sent to the attestation block creation task.
The number of these "blocks" may be conditioned by the backpressure parameter and current congestion conditions (see below)

### Attestation block creation task
Retrieves the historical blocks from the source chain and builds STARK-fashioned Merkle trees.
Maintains a simple retrial mechanism for network failure recovering.
The attestation blocks are then sent to the resiliency priority queue (see below).

### Build attestation chain task
Receives the "blocks" from the purgatory queue, spawns the block creation tasks and sends the crafted attestation blocks to the resiliency queue,
where they wait for the previous blocks (if any) to be ready so the attestations are chained properly.

### Resiliency priority queue
The reason for existence of this data structure is network failures and attempts for recovering.
The fact of re-attempting to retrieve blocks from the source chain invalidates the assumption of the in-order nature of task handles joining.
The resiliency queue gathers all the crafted attestation blocks until there are one or more blocks that can be liberated observing the attestation chain ordering.

### Backpressure
After recovering from a network failure, block creation tasks would accumulate and flood the network with requests, eventually causing the server to deny the service (error 429).
To mitigate this, the purgatory queue is allowed to expulse only a limited number of "blocks" on each cycle depending on how many block creation tasks are currently active.

### Disconnected mode
When the program discovers a steady network failure it sets itself on a low gear and just pings the server once in a while.

### The main wrapper
The above tasks are encapsulated in a single object as an essay to present better ergonomics and provide hooks for handling essential events.

This crate does not provide executables.
Checking network failures:
There is no programmatic network failure simulation, so real internet disconnection is needed.
Even in the best case the program will need some time to stabilize after network reconnection.

## Results
The program's attestation chain building seems to be able to keep up with the source chain block production rate.
This means that it seems to be approximately neither faster nor slower then the Ethereum's block time.
The asynchronous runtime used is the multithreaded Tokio runtime, but it's unclear how the workloads of those threads are internally distributed.
The possible bottlenecks of this program may be
- the slow network API used (one call per single transaction)
- not totally parallelized heavy CPU workloads of building Pedersen Merkle trees (perhaps we could engage parallel rayon primitives manually)

