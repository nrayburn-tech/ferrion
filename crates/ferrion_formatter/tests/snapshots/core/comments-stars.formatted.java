class A {
  /** Stars at different columns all go one column after the slash. */
  int a;

  /*
  * A block comment with an empty line

  * keeps its relative indentation, the least indented star at the comment's column.
  */
  int b;

  /*
  text first
           * then a star
               */
  int c;
}
