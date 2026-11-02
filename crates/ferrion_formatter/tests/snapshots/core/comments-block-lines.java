// Line breaks around one-line block comments: kept between statements and members, and in an
// expression or a list only where the rules break.
class A {
  int a;
  /* before a member */ int b;

  void m() {
    f(0, /* a= */ 0,
        /* b= */ 0, /* c= */ 1000);
    f(0 /* b */
        , 1);
    f(0, /* trailing */
        1);
    int x = a +
        /* c */ b;
    int y =
        /* c */ 5;
    int z = /* c */
        5;
    g(a, b) /* c */
        .h();
    if (a) /* c */
      b();
    while (a) /* c */
      b();
    f(0,
        /* alone */
        1);
    a();
    /* own line */ b();
    a(); /* trailing */
    b();
    if (a) {
      b();
    }
    /* before else */ else {
      c();
    }
  }

  void n(int a,
      /* b */ int b) /* c */
      throws E {}

  int[] array = {1,
      /* two */ 2};
  int q = 1 /* c */
      ;
}
