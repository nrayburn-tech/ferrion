class CatchParameterAnnotations {
  void m() {
    try {
      run();
    } catch (
        @SuppressWarnings("unused")
        IOException e) {
      run();
    } catch (@Foo IOException e) {
    } catch (@Foo @Bar IOException e) {
    } catch (final @SuppressWarnings("unused") IOException e) {
    } catch (
        @SuppressWarnings("unused")
        final IOException e) {
    } catch (@SuppressWarnings("unused") IOException | RuntimeException e) {
    } catch (
        @Foo
        @SuppressWarnings("unused")
        IOException e) {
    } catch (
        @SuppressWarnings("unused")
        @Foo
        final IOException e) {
    } catch (
        @SuppressWarnings("unused")
        IOException eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee) {
    } catch (
        @SuppressWarnings("unused")
        IOException e) { // c
    }
  }
}
