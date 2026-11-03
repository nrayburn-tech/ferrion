class A {
  /** x <a href="u"> link</a> y */
  void a() {}

  /** x <a> link</a> y */
  void b() {}

  /** x <b class="c"> bold</b> y */
  void c() {}

  /** x <tt> t</tt> y */
  void d() {}

  /** x <span id="s"> s</span> y */
  void e() {}

  /**
   * x <a href="u">
   * link</a> y
   */
  void f() {}

  /** x <a name="n"> link</a> y */
  void g() {}

  /** x <A HREF="u"> link</A> y */
  void h() {}

  /** x <a href="u" >  link  </a> y */
  void i() {}

  /** x <b> bold</b> y */
  void j() {}

  /** x <i title="t"> it</i> y */
  void k() {}

  /**
   * word word word word word word word word word word word word word <a href="http://x.com/yy">link text</a> tail
   */
  void wa() {}

  /**
   * word word word word word word word word word word word word word <a href="http://x.com/yy"> link text</a> tail
   */
  void wb() {}

  /**
   * word word word word word word word word word word word word word word word word <a href="u">link</a> tail
   */
  void wc() {}

  /**
   * word word word word word word word word word word word word word word word word w <a href="u"> link</a>
   */
  void wd() {}
}
