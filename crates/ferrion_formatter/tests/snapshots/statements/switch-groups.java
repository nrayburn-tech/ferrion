// Switches with groups, `case …:`: labels, statements, blank lines, and long labels.
class SwitchGroups {
  void m() {
    switch (a) { case 1: case 2: f(); break; default: g(); }
    switch (a) { case 1: { f(); break; } case 2: { g(); } h(); default: }
    switch (a) { case 1, 2: f(); }
    switch (a) { }
    switch (a) { default: }
    switch (a) { case 1: case 2: default: f(); }
    switch (a) { case 1: l: f(); }
    switch (a) {

      case 1:

        f();

        break;

      case 2:
        g();

    }
    switch (a) { case AAAAAAAAAAAAAAAAAAAAAA, BBBBBBBBBBBBBBBBBBBBBBBBBB, CCCCCCCCCCCCCCCCCCCCCCCC, DDDDDDDDDDDDDDDDD: f(); default: }
    switch (o) { case Integer i when i > 0: f(); default: }
    switch (o) { case SomeVeryLongTypeNameNumberOne someLongName when someLongName.someCondition(otherCondition): f(); }
    switch (aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa) { default: }
  }
}
