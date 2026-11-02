// Line breaks around one-line block comments: none after one that trails code, other than an
// opening delimiter, where the rules don't break.
class A {
  int a;
  /* before a member */ int b;

  void m() {
    f(0 /* b */
        , 1);
    f(0, /* trailing */
        1);
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
    if (received < allocated) /* input ran out early, so take what is left */ settledCount = length;
  }

  void n(int a, int b) /* c */
      throws E {}

  int q = 1 /* c */
      ;
}
