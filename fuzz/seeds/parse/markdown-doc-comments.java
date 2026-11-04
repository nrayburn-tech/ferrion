class Fill {
  /// Para   one
  /// continues here.
  ///
  /// Para two word word word word word word word word word word word word word word word word word word word word
  void a() {}

  ///no space
  ///   three spaces
  void b() {}

  ///
  /// leading and trailing blank lines go
  ///
  ///
  ///
  /// doubled blank lines too
  ///
  void c() {}

  ///
  ///
  void d() {}

  /// aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa b
  void w99() {}

  /// aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa b
  void w100() {}

  /// aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa b
  void w101() {}

      ///   Mis-indented
        ///   lines
  void e() {}

  /// A [link   text](http://x.com "a   title") and ![img  alt](i.png) <http://auto.link> <b>a   b</b>
  /// &amp;   \* *emphasis
  /// across*   lines and **strong   text**
  void f() {}

  /// A {@link Foo   bar} and {@code a   b} and [Foo]   and [text][Foo]   here.
  void g() {}

  /// a well-
  /// known thing
  void h() {}

  /// Text ending in a backslash \
  /// next line\
  /// last   line
  void i() {}

  /// trailing spaces at the end  
  void j() {}
}

class Lists {
  /// Intro
  /// - item one
  /// - item two
  ///   continues   here
  /// - item three word word word word word word word word word word word word word word word word word word word word
  ///
  /// 1. first
  /// 2. second
  ///    * nested   one
  void a() {}

  /// * star   item
  /// + plus   item
  ///
  /// 3. three
  /// 7. seven
  ///
  /// - loose one
  ///
  /// - loose two
  /// after   list no blank
  void b() {}

  /// - item
  ///   - nested
  ///     more   text
  ///   - nested two
  ///
  ///     para in nested
  /// - back
  void c() {}

  /// 1) paren   item
  /// 2) two
  ///
  /// 1. ordered item word word word word word word word word word word word word word word word word word word word word
  /// 10. ten item word word word word word word word word word word word word word word word word word word word word
  void d() {}

  /// - item word word word word word word word word word word word word word word word word word word word word
  ///
  ///   second para in item word word word word word word word word word word word word word word word word word word word word
  ///   > quote in item
  /// - next
  void e() {}

  /// - a
  /// - b
  ///
  /// para after list
  void f() {}

  /// 1. one
  ///
  ///    para
  /// 2. two
  void g() {}
}

class Quotes {
  /// > quote word word word word word word word word word word word word word word word word word word word word
  /// > second
  ///
  /// > para one
  /// >
  /// > para two
  void a() {}

  /// Para
  /// > quote right after
  /// lazy   continuation
  ///
  /// after
  void b() {}

  /// > a
  /// > > nested   b
  /// > > more
  void c() {}

  /// > - list in quote word word word word word word word word word word word word word word word word word word word word
  /// >   cont
  void d() {}

  /// > text
  /// > - list after text in quote
  void e() {}

  /// > > a
  void f() {}

  /// > # head in quote
  /// > text
  void g() {}

  /// > ```
  /// > code
  /// > ```
  void h() {}
}

class Blocks {
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

class Tags {
  /// Text.
  /// @param x the x word word word word word word word word word word word word word word word word word word word word
  ///
  ///   second para of x word word word word word word word word word word word word word word word word word word word word
  /// @param y
  /// @throws E when
  ///   - list one
  ///   - list two
  /// @return   r
  int a(int x, int y) {}

  /// @param x only tags
  void b(int x) {}

  /// Text.
  ///
  ///
  /// @see Foo
  ///
  /// @see Bar
  void c() {}

  /// Text.
  /// @param x
  ///     indented continuation
  void d(int x) {}

  /// - item
  /// @param x desc
  void f(int x) {}

  ///
  /// @return r
  int g() {}
}
