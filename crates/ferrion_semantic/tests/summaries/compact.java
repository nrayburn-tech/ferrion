// release: 25
import java.util.List;

final List<String> NAMES = List.of("a");

int count;

void main() {
    System.out.println(NAMES);
}

static String greet(String name) { return "hi " + name; }

record Pair(int a, int b) {}

class Helper {}
