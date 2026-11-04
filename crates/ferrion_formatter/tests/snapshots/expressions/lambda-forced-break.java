// A lambda with an expression body that holds a forced break doesn't go on a line of its own: it
// stays on the call's line, and breaks after its arrow.
class T {
  void m() {
    AsyncTask delivery = async(() -> engine.dispatch("orders", MessageBatch.create(List.of(message("order"))), () -> { events.add("delivery-start"); }));
    async(() -> engine.dispatch("orders", MessageBatch.create(List.of(message("order"))), () -> { events.add("delivery-start"); }));
  }
}
