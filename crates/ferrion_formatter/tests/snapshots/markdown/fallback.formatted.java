class A {
  /// Para   one
  ///
  /// Setext
  /// ------
  void a() {}

  /// Before   indented
  ///
  ///     indented   code
  ///
  /// After   indented
  void b() {}

  /// text   a
  ///
  /// ***
  /// text   b
  void c() {}

  /// [a]: http://example.com   "title"
  /// text   [a]
  void d() {}

  /// Text   then {@code unclosed
  /// more
  void e() {}

  /// x   y
  /// Setext
  /// ---
  void f() {}
}
