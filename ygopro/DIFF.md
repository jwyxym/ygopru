# Difference with ygopro
ygopru does its best to keep the binary message protocol and its timing identical to ygopro. But for extensibility and implementation details, some messages differ from the original, hence this document.

+ When duel is created, a `CreateGame` message will be sent to duel. That message doesn't exist in the original ygopro, it is for init necessary duel data.
+ The `ygocore` duel is created when the room is created (`DuelHost::new`), unlike the original which creates it in `TpResult`.
+ When net player leave, a `LeaveGame` message will be sent to duel. That message doesn't exist in the original ygopro, it is only for plugin can catch that time.
