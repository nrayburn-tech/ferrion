class Records {
  record A(int a, int b) {}

  record B() {}

  record C(
      int aaaaaaaaaaaaaaaaa, String bbbbbbbbbbbbbbbbbbbbbb, List<String> cccccccccccccccccccccc) {}

  record D(
      int aaaaaaaaaaaaaaaaa,
      String bbbbbbbbbbbbbbbbbbbbbb,
      List<String> cccccccccccccccccccccc,
      int ddddddddddd,
      int eeeeeeeeeeeeeeeeeeeeeeeeeee) {}

  record E<T>(@Nullable T a, String... rest) implements Comparable<E<T>> {
    E {
      Objects.requireNonNull(rest);
    }

    public E {}

    static int x;
  }

  public record F(int aaaaaaaaaaaaaaaaa, String bbbbbbbbbbbbbbbbbbbbbb)
      implements Cccccccccccccccccccccccc {}

  record G(
      int aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,
      int bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,
      int cccccccccccccccc)
      implements C {}

  record H(
      @Deprecated int aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,
      @A(1) int bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb) {}

  record I(int a, int b)
      implements Cccccccccccccccccccccccccccccccccccccccccccccccc, Ddddddddddddddddddddddd {}

  record J(int a) {
    int b() {
      return a;
    }

    static int c;
  }
}
