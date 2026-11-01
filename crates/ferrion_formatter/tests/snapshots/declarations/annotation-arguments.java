class AnnotationArguments {
  @A(value = "aaaaaaaaaaaaaaaaaaaaaaaaaa", other = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", third = 3) void a() {}
  @A({"aaaaaaaaaaaaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "ccccccccccccccccccccccc", "dddd"}) void b() {}
  @A(names = {"aaaaaaaaaaaaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "ccccccccccccccccccc", "dddd"}) void c() {}
  @A(names = {@B("aaaaaaaaaaaaaaaaaaaaaaaaaa"), @B("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"), @B("ccccccc")}) void d() {}
  @A() void e() {}
  @A({}) void f() {}
  @A({1, 2,}) void g() {}
  @a.b.C(x = 1) void h() {}
  @A(@B) void i() {}
  @A("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa") void j() {}
  @A(b = @B(ccccccccccccccccccccccccccccccc = 1, dddddddddddddddddddddddddddddddd = 2), e = "eeeeeeee") void k() {}
  @A(@B(ccccccccccccccccccccccccccccccc = 1, ddddddddddddddddddddddddddddddddddddddddddddddddddd = 2)) void l() {}
  @A(value = {"a", "b"}, other = {"c"}) void m() {}
  @A({"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}) void n() {}
  @A({1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28}) void o() {}
  @A(x = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27}) void p() {}
  @A(x = 1, // c
      y = 2) void q() {}
  @A(x = {
    1, 2
  }) void r() {}
  @A({ // c
      1}) void s() {}
  @A({{1}}) void t() {}
  @A({"aaaaaaaa", "bbbbbbbb", "cccccccc", "dddddddd", "eeeeeeee", "ffffffff", "gggggggg", "hhhhhhhh", "i"}) void u() {}
  @A({"aaaaaaa", "bbbbbbb", "ccccccc", "ddddddd", "eeeeeee", "fffffff", "ggggggg", "hhhhhhh", "iiiiiii", "j"}) void v() {}
  @A({Aaaaaaaaaaa.class, Bbbbbbbbbbbbbb.class, Cccccccccccccc.class, Dddddddddddddd.class, Eeeeeeeeeeee.class}) void w() {}
  @A({X.class, Y.class, Z.class, W.class, V.class, U.class, T.class, S.class, R.class, Q.class, P.class}) void x() {}
  @A({-1, -2, -3, -4, -5, -6, -7, -8, -9, -10, -11, -12, -13, -14, -15, -16, -17, -18, -19, -20, -21, -22}) void y() {}
  @A(x = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa") void z() {}
}
