enum Empty {}

enum OnlyComma { , }

enum OnlySemicolon { ; }

enum Color { RED, GREEN, BLUE, }

enum Planet implements Body {
  @Deprecated MERCURY(3.3e23, 2.4e6),
  EARTH(5.9e24, 6.3e6) {
    @Override double gravity() { return 9.8; }
  };

  private final double mass;

  Planet(double mass, double radius) {}

  double gravity() { return 0; }
}
