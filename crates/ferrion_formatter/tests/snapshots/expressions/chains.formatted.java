// Method chains: with two or more calls, each link after the prefix breaks at +4, all at once;
// with one call, the dots fill and the arguments break first.
class Chains {
  void m() {
    x.y().z();
    builder
        .append(argumentNumberOneAAAAAAAAAAA)
        .append(argumentNumberTwoBBBBBBBBBBBBB)
        .append(cccccc);
    abcd.append(argumentNumberOneAAAAAAAAAAA)
        .append(argumentNumberTwoBBBBBBBBBBBBB)
        .append(ccccccccc);
    this.builder
        .append(argumentNumberOneAAAAAAAAAAA)
        .append(argumentNumberTwoBBBBBBBBBBBBB)
        .append(c);
    super.foo(argumentNumberOneAAAAAAAAAAA)
        .baz(argumentNumberTwoBBBBBBBBBBBBB)
        .qux(argumentNumberThree);
    Outer.this
        .foo(argumentNumberOneAAAAAAAAAAA)
        .baz(argumentNumberTwoBBBBBBBBBBBBB)
        .qux(argumentNumber);
    ImmutableList.<String>builder()
        .add(argumentNumberOneAAAAAAAAAAA)
        .add(argumentNumberTwoBBBBBBBBBBBBB)
        .build();
    java.util.Objects.requireNonNull(foo)
        .bar(argumentNumberOneAAAAAAAAAAA)
        .baz(argumentNumberTwoXXXX);
    Foo.bar
        .baz(argumentNumberOneAAAAAAAAAAA)
        .qux(argumentNumberTwoBBBBBBBBBBBBB)
        .append(cccccccccccc);
    FOO_BAR
        .of(argumentNumberOneAAAAAAAAAAA)
        .bar(argumentNumberTwoBBBBBBBBBBBBB)
        .bar(argumentNumberThree);
    Foo.class
        .getName()
        .aaaaaaaaaaaaaaaaaaaaaaaaaaa()
        .bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb()
        .cccccccc();
    assertThat(someValueAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA)
        .isEqualTo(expectedValueBBBBBBBBBBBBBBBBBBBBBBBB);
    new Foo()
        .bar(argumentNumberOneAAAAAAAAAAA, argumentNumberTwoBBBBBBBBBBBBB, argumentNumberThreeCCCC);
    "abc"
        .concat(
            argumentNumberOneAAAAAAAAAAA,
            argumentNumberTwoBBBBBBBBBBBBB,
            argumentNumberThreeCCCCCCC);
    foo.bar(
            argumentNumberOneAAAAAAAAAAA,
            argumentNumberTwoBBBBBBBBBBBBB,
            argumentNumberThreeCCCCCCCCCC)
        .baz();
    foo.bar(argumentNumberOneAAAAAAAAAAA)
        .baz(
            argumentNumberTwoBBBBBBBBBBBBB,
            argumentNumberThreeCCCCCCCCCC,
            argumentNumberFourDDDDDDDDDDDDDD);
    foo.bar.baz.qux(
        argumentNumberOneAAAAAAAAAAA, argumentNumberTwoBBBBBBBBBBBBB, argumentNumberThreeCC);
    this.someField.someMethod(
        argumentNumberOneAAAAAAAAAAA, argumentNumberTwoBBBBBBBBBBBBB, argumentThree);
    aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
        .bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb(x);
    a.b.c.d.eeeeeeeeeeeeeeeeeeeeeeeeeeeeeee.ffffffffffffffffffffffffffffff
        .gggggggggggggggggggggggggg.hhhhhhhhhhhhh();
    x =
        aaaaaaaaaaaaaaaaaaaa
            .bbbbbbbbbbbbbbbbbbbbbbbbbbbbb
            .ccccccccccccccccccccccccccccccc
            .dddddddddddddddddddddddd;
    someVeryLongName.entrySet().stream()
        .sorted(aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa)
        .forEach(bbbbbbbbbbbbbbbbbbbbbbbbbb);
    someVeryLongName
        .entrySet()
        .iterator()
        .sorted(aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa)
        .forEach(bbbbbbbbbbbbbbbbbbbbbbbbb);
    it
        .lookupMetadata(Kinds.BLUEPRINT_REQUESTED)
        .flatMap(Descriptor::labelEntries)
        .orElseGet(List::of)
        .stream()
        .map(String::trim)
        .forEach(descriptors::add);
    x =
        foo.bar(argumentNumberOneAAAAAAAAAAA)
            .baz(argumentNumberTwoBBBBBBBBBBBBB, argumentNumberCCCCCC);
    foo(
        aaaaaaaaaaaaaaaaaaaaaaaaaa
            .bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb()
            .ccccccccccccccccccccccccccccccccc());
    a.b()[0]
        .c(argumentNumberOneAAAAAAAAAAA)
        .d(argumentNumberTwoBBBBBBBBBBBBB)
        .e(ccccccccccccccccccc);
    builder
        .a() // trailing a link
        .b();
    builder
        .a()
        // between links
        .b();
  }
}
