class Parameters {
  void a(
      @Nullable String aaaaaaaaaaaaaaaa,
      final @Deprecated int bbbbbbbbbbbbbbbbb,
      @A(1) int... ccccccccc) {}

  void b(@A @B @C int a, @B("x") final int b) {}

  void c(
      @A(x = 1, y = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
          int a) {}

  void d(
      @Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
          @Bbbbbbbbbbbbbbbbbbbbbbbbbbb
          int a) {}

  void e(
      @Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa(xxxxxxxxxxxxxxxxxxxx = 1) @Bbbbbbbbbbbbbbbbbbbbbbbbb
          final int a) {}

  void f(
      int
          aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa) {}

  void g(
      int...
          aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa) {}

  void h(
      final Parameters this,
      int aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,
      int bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb) {}

  class Inner {
    Inner(Parameters Parameters.this) {}
  }
}
