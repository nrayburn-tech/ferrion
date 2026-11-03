class A {
  /**
   * Before.
   * <pre>
   *   int x  =  1;
   *      @Override   y
   *
   *   z
   * </pre>
   * After pre.
   * <pre>{@code
   * Map<K, V> m = new  HashMap<>();
   *   if (a) { b(); }
   * }</pre>
   * Tail.
   */
  void a() {}

  /**
   * <table>
   *   <tr><td>a   b</td>
   *  <td>c</td></tr>
   * </table>
   * After table.
   */
  void b() {}

  /**
   * Text <blockquote>quoted
   * text</blockquote> more.
   * <h2>Heading   two</h2>
   * After heading.
   * Line<br>break and<br/>
   * more.
   * <!-- a comment
   *   spanning -->
   * After comment.
   * <foo>unknown</foo> <DIV>div</DIV> <dl><dt>t<dd>d</dl>
   */
  void c() {}

  /**
   * <pre>
   * x</pre>
   * <pre>   lead</pre>
   */
  void d() {}
}
