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
    when(remoteClient.dispatch(
            anyString(), eq(TaskStatus.DONE), any(JobRequest.class), eq(String.class)))
        .thenReturn(cannedResponse);
    assertThat(
            someObject.someMethod(
                argumentNumberOneAAAAAAAAAAAAAAAA, argumentNumberTwoBBBBBBBBBBBBBBB))
        .isEqualTo(expected);
    when(remoteClient
            .dispatch(anyString(), eq(TaskStatus.DONE), any(JobRequest.class), eq(String.class))
            .foo())
        .thenReturn(cannedResponse);
    it.map(String::trim) // trailing a call
        .filter(Predicate.not(String::isBlank));
    builder
        .a() // trailing a link
        .b();
    builder
        .a()
        // between links
        .b();
  }

  // With one call, the dot after a field that follows a type's name breaks if the one before
  // the field does.
  void typeThenField() {
    RepositoryQueryMethodBenchmarks_BenchmarkParameters_jmhTypeAAAAAAAAAAAAAAAAAAAA
        .setupIterationMutexUpdater
        .f2.f3.get(l_benchmarkparameters1_G);
    RepositoryQueryMethodBenchmarks_BenchmarkParameters_jmhTypeAAAAAAAAAAAAAAAAAAAA
        .setupIterationMutexUpdater
        .get(l_benchmarkparameters1_G, aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa);
    while (RepositoryQueryMethodBenchmarks_BenchmarkParameters_jmhType.setupIterationMutexUpdater
            .get(l_benchmarkparameters1_G)
        == 1) {}
    org.example.RepositoryQueryMethodBenchmarks_BenchmarkParameters_jmhType
        .setupIterationMutexUpdaterAAAAAAA
        .get(l_benchmarkparameters1_G);
    RepositoryQueryMethodBenchmarksBenchmarkParametersJmhTypeAAAAAAAA.Inner
        .setupIterationMutexUpdater
        .get(l_benchmarkparameters1_G);
    repositoryQueryMethodBenchmarks_BenchmarkParameters_jmhTypeAAAAAAAAAAAAAAAAAAAA
        .setupIterationMutexUpdater.get(l_benchmarkparameters1_G);
    ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOP
        .setupIterationMutexUpdater.get(l_benchmarkparameters1_G);
  }

  // A chain's only call, with fields after it, is in the prefix, and so is a `.stream(...)`,
  // each breaking from the root only if the line up to its `(` doesn't fit.
  void singleCall() {
    result =
        lookup.lookup(
                span("name", "a", "z", T, T, RangeSelection.INCLUSIVE_BOUNDS_FAST_ITERATOR), 1000)
            .hitsFound;
    result =
        lookup.lookup(span("name", "a", "z", T, T, RangeSelection.INCLUSIVE_BOUNDS_EXACT), 1)
            .hitsFound
            .length;
    byte[] bytes =
        contentRef.getContent()
            .bytes; // safe here to just use the bytes, otherwise we should use offset, length
    byte[] bytes =
        contentRef
            .getContentAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA()
            .bytes;
    best =
        a.b.closest(openSession(r, false), "place", 2, 45.0f, 50.0f, aaaaaaaaaaaaaaaaaaaaaaa)
            .hitsFound;
    codePointWidth =
        foooooAAAAAAAAAAAAAAAAAAAAA.stream(
                aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbbbb)
            .flatMapToInt(Arrays::stream)
            .toArray();
    x =
        explorerAAAAAAAAAAAAAAAAAA
            .lookup(span("name", "a", "z", T, T, RangeSelection.EXCLUDED), 1000)
            .hitsFound
            .length
            .foo();
  }

  // A call's type arguments stay with the dot, and the name breaks after them at +4.
  void typeArguments() {
    legs =
        this
            .<ShipmentRoutingRegressionIT.LegRecord, QShipmentRoutingRegressionIT_LegRecord>
                createList(
                    "legs",
                    ShipmentRoutingRegressionIT.LegRecord.class,
                    QShipmentRoutingRegressionIT_LegRecord.class,
                    PathInits.DIRECT2);
    legs =
        this.<ShipmentRoutingRegressionIT.LegRecord, QShipmentRouti>createList(
            "legs", ShipmentRoutingRegressionIT.LegRecord.class, PathInits.DIRECT2);
    foo.bar()
        .<ShipmentRoutingRegressionIT.LegRecord, QShipmentRoutingRegressionIT_LegRecordAAAAAA>
            createList(
                "legs",
                ShipmentRoutingRegressionIT.LegRecord.class,
                QShipmentRoutingRegressionIT_LegRecord.class,
                PathInits.DIRECT2)
        .baz();
    this
        .<ShipmentRoutingRegressionIT.LegRecord, QShipmentRoutingRegressionIT_LegRecord,
            ShipmentRoutingRegressionIT.LegRecord2, ShipmentRoutingRegressionIT.LegRecord3>
            createList("legs");
  }
}
