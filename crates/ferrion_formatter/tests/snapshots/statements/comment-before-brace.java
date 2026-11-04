// A `}` stays on the line of a block comment that starts its line, unless the block is an `if`'s
// without an `else`, or it ends a lambda in a call.
class T {
  void a() {
    try { x(); } catch (Exception ignore) {
      /* best-effort */ }
    if (a) {
      /* c */ }
    if (a) {
      x();
    } else {
      /* d */ }
    while (a) {
      /* c */ }
    for (;;) {
      /* c */ }
    {
      /* c */ }
    y();
    /* e */ }
  void b() {
    /* c */ }
  void c() { /* c */ }
  void d() {
    r(() -> {
      /* c */ });
  }
  class C {
    /* c */ }
  class D {
    int x;
    /* c */ }
  interface G {
    /* c */ }
}
