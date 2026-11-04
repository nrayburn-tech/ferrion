// When the group up to a `.stream()` doesn't fit, a type's name keeps its first link.
class T {
  void m() {
    Optional.ofNullable(properties.getProperty("in")).map(it -> it.split(",")).map(List::of).stream().flatMap(List::stream).forEach(builder::addSomethingLongEnough);
    Optional.ofNullable(properties.getProperty("in")).map(it -> it.split(",")).map(List::of).stream();
    Arrays.asList(firstLongArgumentNameThatIsLong, secondLongArgumentNameThatIsLong, thirdLongArgument).stream().map(x).toList();
    foo.bar(firstLongArgumentNameThatIsLong, secondLongArgumentNameThatIsLong, thirdLongArgument).baz().stream().map(x).toList();
  }
}
