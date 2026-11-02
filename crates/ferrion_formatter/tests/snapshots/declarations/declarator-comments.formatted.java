class DeclaratorComments {
  int a // c
      ,
      b;
  int[] chars // only the first
      = {1, 2};
  int x // c
      = 1;
  int y // c
      =
      foo(
          aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,
          bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,
          ccccc);

  void m() {
    int a // c
        ,
        b;
    int[] chars // only the first
        = {1, 2};
    int x // c
        = 1;
  }
}
