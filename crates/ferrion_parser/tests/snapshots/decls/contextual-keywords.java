// release: 8
class A {
  // At Java 8, `sealed`, `non`, `permits`, `module`, and `open` are only names, and so is
  // `record` where no name follows it.
  sealed s;
  int permits;
  int non, sealed;
  record[] records;
  Object record() { return null; }
  module open;
}

class sealed {}
