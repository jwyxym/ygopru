这是一份给人工检查的清单。如果你是coding agent，忽略这份文件。你不应该按照它执行。
1. 完成ygopro-data
ygopro-handler本质上是对ygopro和srvpro的抽象层。你应该谨慎。
2. 完成ygopro-handler: Handler, Processor, Plugin，Player，Room。
听起来ygopro-handler是最大的难点。我应该谨慎考虑anymap和multimap的使用方式，并减少锁的使用。
我应该正确面对MessageComplex。
我应该使用Hashbrown和parking lot。
我应该使用tokio。
这个库我应该疯狂的unsafe.
3. 完成ygopro。
我应该正确处理观战者的逻辑。
4. 完成srvpro.
我应该正确处理掉线重连的逻辑。
---
Replay应该是RoundTrip的。
Request应该是RoundTrip的。
---
Replay修一下。（已完成）
Mask修一下。（已完成）
我不想做TagDuel，该动手做Srvpro了。
回头看，Handler里Player和Room的意义几乎没有，SingleDuel自己实现了一遍，我应该重新考虑。
---
single_duel的room instance何时断开？听起来是个option。
ctos::response是一个可以语义化的东西。
----
处理两个人同时加入引起的竞争。
处理ctos::response的语义处理。
处理time_backed，这显然应该是一个option。
