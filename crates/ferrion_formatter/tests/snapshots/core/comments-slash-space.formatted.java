// A space goes after the slashes of a line comment
class A {
  //
  //
  ///
  ////
  /// a Markdown doc comment
  ///without a space
  ////four
  // -----
  // ====
  // !bang
  // #hash
  // *star
  // TODO fix
  // NOSONAR
  //	tab
  // already
  //noinspection SomeInspection
  //$NON-NLS-1$
  // $NON-NLS
  void m() {
    int x = 1; // trailing
    f(); //noinspection SomeInspection
    // $FALL-THROUGH$
    g(
        a, // after a comma
        b);
  }
}
