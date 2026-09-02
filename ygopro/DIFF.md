# Difference with ygopro
ygopru does its best to keep the binary message protocol and its timing identical to ygopro. But for extensibility and implementation details, some messages differ from the original, hence this document.

+ When duel is created, a `CreateGame` message will be sent to duel. That message doesn't exist in the original ygopro, it is for init necessary duel data.
+ The `ygocore` duel is created when the room is created (`DuelHost::new`), unlike the original which creates it in `TpResult`.
+ When net player leave, a `LeaveGame` message will be sent to duel. That message doesn't exist in the original ygopro, it is only for plugin can catch that time.

## MessageEx
For extensibility, ygopru introduces a set of internal messages (MessageEx) so it can hook into and alter parts of ygopro's behavior without occupying the message flags of the ygopro protocol. They are only exchanged inside the duel actor and are never sent over the network.

- `ClientJoin`: when a new client attaches to the room.
- `FirstShuffle`: shuffle both players' main decks in first-attack order right after `TpResult`.
- `DuelInit`: load both decks into the core with new_card.
- `DuelStart`: send init field info (deck + extra), then evolve the ygocore.
- `DuelEnd`: when a duel is ended.
- `GenerateReplay`: when server needs send replay to client.
- `JudgeContinueMatch`: decide whether the match should continue; continue leads to `RecreateDuel`, terminate leads to `MatchEnd`.
- `RecreateDuel`: enter siding, reset player states and recreate the ygocore duel.
- `MatchEnd`: when a match is ended.
- `Terminate`: when the room is dropped.

The duel start is driven by this chain:

    TpResult -> FirstShuffle
             -> DuelInit -> DuelStart -> Evolve

The duel end is driven by this chain:

    DuelEnd -> GenerateReplay
            -> JudgeContinueMatch -> RecreateDuel   (match continues)
                                  -> MatchEnd       (match ends)
