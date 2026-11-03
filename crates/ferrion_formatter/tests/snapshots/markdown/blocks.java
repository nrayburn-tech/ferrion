class A {
  /// Before fence
  /// ```java
  /// int   x;
  ///    y();
  /// ```
  /// After fence
  void a() {}

  ///x   y
  ///```
  ///int  z;
  ///  w();
  ///```
  void b() {}

  ///   x   y
  ///   ```
  ///   int  z;
  ///     w();
  ///   ```
  void c() {}

  /// # Heading   one
  /// text   under
  /// ## Two ##
  /// #    Heading   with   spaces   ##
  void d() {}

  /// | a | b |
  /// |---|---|
  /// | 1 | 2 |
  /// text   x
  void e() {}

  /// Before   t
  /// | a  |  b |
  /// |--|--|
  ///
  /// after   t
  void f() {}

  /// text
  /// <pre>
  /// x   y
  /// </pre>
  /// text two
  void g() {}

  /// <!-- html   comment -->
  /// text   x
  void h() {}

  /// ~~~
  /// tilde   fence
  /// ~~~
  /// ````
  /// four ``` inner
  /// ````
  void j() {}

  /// a
  /// ```
  /// unclosed fence
  void k() {}

  /// # head
  /// - list after head
  /// # head after list
  void l() {}

  /// - item
  ///   ```
  ///   code
  ///   ```
  /// - next
  void m() {}

  /// - a
  ///
  /// # head
  void n() {}
}
