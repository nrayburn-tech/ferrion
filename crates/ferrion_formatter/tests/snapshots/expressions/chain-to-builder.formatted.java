// `.toBuilder()` is kept with what comes before it, as `.stream()` is, and the last of them ends
// the prefix.
class T {
  void m() {
    x =
        Configuration.unix().toBuilder()
            .setRoots("/", "/foo", "/bar")
            .setWorkingDirectory("/work/dir")
            .build();
    y =
        descriptor.toBuilder()
            .setRoots("/", "/foo", "/bar")
            .setWorkingDirectory("/work/dir")
            .setAnother(a)
            .build();
    fooooooo.stream().toBuilder()
        .map(it -> it.split(","))
        .map(List::of)
        .flatMap(List::stream)
        .forEach(xyzabcdefgh);
    fooooooo.toBuilder().stream()
        .map(it -> it.split(","))
        .map(List::of)
        .flatMap(List::stream)
        .forEach(xyzabcdefgh);
    new Foo(x)
        .toBuilder()
            .map(it -> it.split(","))
            .map(List::of)
            .flatMap(List::stream)
            .forEach(xyzabcdefghiabcdefgh);
    fooooooooooooooooooooooooooooooooooooo.toBuilder(
            aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa)
        .build();
    foo.toBuilder().build();
  }
}
