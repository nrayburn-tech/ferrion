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

  /// # Headin2147483648g   one
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
  /// next
  void m() {}

  /// - a
  ///
  /// # head
  void n() {}
}
