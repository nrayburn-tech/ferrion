class A {
  /**
   * <ul>
   * <li>Item with pre:
   * <pre>
   *   code
   * </pre>
   * <li>Item with table <table><tr><td>x</td></tr></table> after
   * </ul>Text right after.
   * <li>stray li
   * <ul>no li inside</ul>
   */
  void a() {}

  /**
   * <blockquote><p>Quoted para.
   * <p>Second.</blockquote>
   * {@code foo
   * @Bar baz}
   * {@code word word word word word word word word word word word word word word word word word @Bar}
   */
  void b() {}

  /**
   * Text <pre>inline pre</pre> more. <h1>Inline h1</h1> more. <ol start="3"><li>x</ol>
   */
  void c() {}

  /**
   * <ul>
   *   <li>a
   *     <ol>
   *       <li>b
   *         <ul>
   *           <li>c word word word word word word word word word word word word word word word word word word
   *         </ul>
   *     </ol>
   * </ul>
   */
  void d() {}

  /**
   * <dl>
   * <dt>Term
   * <dd>Def word word word word word word word word word word word word word word word word word word word
   * </dl>
   * <menu><li>x</menu>
   */
  void e() {}
}
