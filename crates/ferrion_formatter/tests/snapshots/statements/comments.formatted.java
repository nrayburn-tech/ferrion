// Comments in and around statements.
class Comments {
  void m() {
    int x = 1; // trailing
    // own line

    // after a blank line
    x++;
    /* block */ x++;
    { // on the brace's line
    }
    {
      /* alone */
    }
    {
      // alone on its line
    }
    while (a) {
      /* alone */
    }
    for (Object o : list) {
      // alone on its line
    }
    if (a) // after the condition
    b();
    if (a)
      // before the body
      b();
    if (a) /* before the body */ b();
    if (a) {
      b();
    } // after the block
    else {
      c();
    }
    if (a) {
      b();
    }
    // before else
    else {
      c();
    }
    if (a) b(); // after the statement
    else c(); // after the else
    if (a) b();
    else // after else
    c();
    while (a) // after the condition
    {
      b();
    }
    do // after do
    b();
    while (a);
    for (int i = 0; i < n; i++) // after the header
    b();
    label: // after the label
    for (; ; ) {}
    try {
      b();
    } // after the block
    catch (E e) {
      c();
    }
    try (R r = open(); // after the resource
        S s = open()) {}
  }

  void switches() {
    switch (a) { // on the brace's line
    }
    switch (a) {
        // alone
    }
    switch (a) {
      // before the first case
      case 1:
        // before the statement
        f();
      // before default
      default:
        g();
        // after the last statement
    }
    switch (a) {
      case 1 /* in the label */:
        f();
      case /* in the label */ 2:
    }
    switch (a) {
      case 1 -> f(); // after the rule
      // before a rule
      case 2 -> /* after the arrow */ g();
      case 3 -> // after the arrow
          h();
      case 4 -> i();
        // after the last rule
    }
  }

  void blockCommentsBeforeALineBreak() {
    a(); /* stays on the line before */
    b();
    {
      /* after the brace, on its own line */
      a();
    }
    if (a) /* stays after the condition */
      ;
    label: /* stays after the label */
    x++;
    try (R r = open(); /* stays after the semicolon */
        S s = open()) {}
    switch (a) {
      /* after the brace */
      case 1: /* stays after the colon */
        f();
    }
    switch (a) {
      case 1 // in the label
          ->
          f();
      case 2 -> f(); /* stays after the rule */
      case 3 -> g();
    }
    int x = 1, // after the comma
        y = 2;
  }
}
