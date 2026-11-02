// Spaces around comments on the line of code, and a bare line comment at the column limit.
class C {
  void m() {
    f(/*c*/ x);
    f(/*c*/ x);
    a[ /*c*/0] = 1;
    x./*c*/ y();
    f(a, /*c*/ b);
    f(a /*c*/, b);
    f(- /*c*/a);
    g(x) /*c*/.h();
    List</*c*/ String> l;
    f(x); // c
    if (a) { // c
    }
    f( // c
        x);
    f(/* none */ );
    f(/* none */ );
    int x = aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbb; //
    int x =
        aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbb; //
    int x =
        aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbb; //
  }
}
