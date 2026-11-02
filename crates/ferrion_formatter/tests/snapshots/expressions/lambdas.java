// Lambdas: one that is a call's only argument is hugged, `foo(x -> {` on the call's line; one
// with an expression body goes on the next line if it fits there, and otherwise breaks after
// its arrow.
class Lambdas {
  void m() {
    executor.submit(() -> { doSomething(); });
    someVeryLongReceiverName.someVeryLongMethodNameThatTakesALambda((firstParameter, secondParameter) -> { doSomething(); });
    foo(a, b, x -> { doSomething(); });
    foo(() -> {});
    foo((String a, int b) -> a + b);
    foo(x -> y -> { run(); });
    foo(bar(x -> { run(); }));
    x = foo(a -> { run(); });
    x = a + bar(x -> { run(); });
    foo.bar(x -> { run(); }).baz();
    foo(a).bar(b).baz(x -> { run(); });
    list.stream().map(x -> x.toString()).filter(x -> { return x.isEmpty(); }).collect(Collectors.toList());
    new Thread(() -> { run(); }).start();
    foo(new ClockProviderParameters(() -> { return time; }));
    list.forEach(x -> System.out.println(xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx));
    list.forEach(x -> System.out.println(xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx, yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy));
    list.forEach(x -> xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx + yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy);
    list.stream().map(x -> System.out.println(xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx, yyyyyyyyyyyyyyyyyyyyyyyyyyyyyy)).collect(toList());
    new Foo(x -> System.out.println(xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx, yyyyyyyyyyyyyyyyyyyyyyyy));
    x = foo(x -> System.out.println(xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx, yyyyyyyyyyyyyyyyyyy));
    foo(x -> x.aaaaaaaaaaaaaaaaaaaaaaaaaa().bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb().cccccccccccccccccccccccccc());
    foo(x -> switch (x) { case 1 -> 2; default -> 3; });
    callSomething(argumentNumberOneAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA, x -> System.out.println(x));
    r = () -> doSomethingVeryLong(argumentNumberOneAAAAAAAAAAA, argumentNumberTwoBBBBBBBBBBBBBBBBBBBBBBB);
    r = () -> { run(); };
  }

  // In a chain's prefix, a lambda with an expression body is hugged, and one with a block isn't.
  void prefix() {
    gen.withArgument(configuredRetryRule -> configuredRetryRule.name("retryRuleArg").type(TypeSpec.builder().from(TypeSpecs.OPTIONAL).addGenericParam(RETRY_RULE_TYPE).build()).attachMarker1(markerForTarget(name))).appendCode("this.retrier = retryRuleArg.orElseGet(").appendCode(generatedType);
    gen.withArgument(configuredRetryRule -> configuredRetryRule.name("retryRuleArg").type(TypeSpec.builder())).appendCode("this.retrier = retryRuleArg.orElseGet(").appendCode(generatedType);
    gen.withArgument(configuredRetryRule -> { foo(); }).appendCode("this.retrier = retryRuleArg.orElseGet(").appendCode(generatedType);
    withArgument(configuredRetryRule -> foo(configuredRetryRuleAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA, bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, ccccccccccccccccccc)).appendCode("this.retrier = retryRuleArg.orElseGet(").appendCode(generatedType);
  }
}
