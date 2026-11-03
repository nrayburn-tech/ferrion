class A {
  /// > quote word word word word word word word word word word word word word word word word word
  /// > word word word second
  ///
  /// > para one
  /// >
  /// > para two
  void a() {}

  /// Para
  /// > quote right after lazy continuation
  ///
  /// after
  void b() {}

  /// > a
  /// > > nested b more
  void c() {}

  /// >
  /// > - list in quote word word word word word word word word word word word word word word word
  /// >   word word word word word cont
  void d() {}

  /// > text
  /// > - list after text in quote
  void e() {}

  /// >
  /// > > a
  void f() {}

  /// >
  /// >
  /// > # head in quote
  /// >
  /// > text
  void g() {}

  /// >
  /// >
  /// > ```
  /// > code
  /// > ```
  void h() {}
}
