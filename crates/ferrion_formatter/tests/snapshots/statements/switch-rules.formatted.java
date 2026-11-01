// Switches with rules, `case … ->`, as statements and expressions: labels, guards, and bodies.
class SwitchRules {
  int m() {
    switch (a) {
      case 1 -> f();
      case 2, 3 -> {
        g();
      }
      default -> throw new E();
    }
    switch (a) {
      case 1 -> {}
      default -> {}
    }
    switch (o) {
      case Integer i when i > 0 -> f();
      case String s -> g();
      case null, default -> h();
    }
    switch (a) {
      case 1 -> f();

      case 2 -> g();
    }
    switch (a) {
      case 1 ->
          someObject.someMethodName(argumentNumberOne, argumentNumberTwo, argumentNumberThree);
    }
    switch (a) {
      case AAAAAAAAAAAAAAAAAAAAAAAAAAA, BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB ->
          someObject.someMethodName(argumentNumberOne);
    }
    switch (a) {
      case AAAAAAAAAAAAAAAAAAAAAA,
          BBBBBBBBBBBBBBBBBBBBBBBBBB,
          CCCCCCCCCCCCCCCCCCCCCCCC,
          DDDDDDDDDDDDDDDDD ->
          f();
    }
    switch (a) {
      case AAAAAAAAAAAAAAAAAAAAAA,
          BBBBBBBBBBBBBBBBBBBBBBBBBB,
          CCCCCCCCCCCCCCCCCCCCCCCC,
          DDDDDDDDDDDDDDDDD -> {
        f();
      }
    }
    switch (o) {
      case SomeVeryLongTypeNameNumberOne someLongName
          when someLongName.someCondition(otherCondition) ->
          f();
    }
    return switch (a) {
      case 1 -> 2;
      default -> {
        int q = 3;
        yield q;
      }
    };
  }
}
