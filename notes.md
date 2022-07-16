

# how flock's algorithm works

## scores

scores are glicko-2 scores, with a rotating 10-rating window for both
user tag and link tag scores, but a decay window of a year for link tag
scores and a decay window of a month (30 days) for user tag scores

decays are checked lazily; that is, they are only performed when a score
is requested (such as when something is liked, disliked, or when a feed
is updated and the score needs to be checked for a link

## feeds

feeds are determined with the following algorithm:

1.  select every link that is both unseen by the user and has a tag
    selected for inclusion by the user

2.  for each link, intersect the tags of the user and the link and

    i.  sum the scores of all of the user's tags and convert each tag's
        score to a percentage of the sum

    ii. take the link's tags and multiply each of them by their
        corresponding tag percentage calculated from the user's tag

    iii. average the resultant scores, creating the "relative overall
         score" of the link

3.  sort the links by their overall score and divide the links into four
    segments, and pick randomly, starting from the top, 4 links, 3
    links, 2 links, and 1 link. this is the user's feed

## auxiliary

### averaging glicko-2 scores

just run the `avg` function over all of the fields

### sorting glicko-2 scores

bucket by value, sort (from top) by decreasing confidence

so, if a player had a rating of 1500, dev of 350, and a volatility of
.6, and another had a rating of 1500, a dev of 600, and a volatility of
.6, the first player would be placed above the second

you sort starting with the rating (which rating is higher), then go to
the deviation (which deviation is smaller), and then go to volatility
(which volatility is smaller)

the reasoning is that smaller values for the latter two parameters
indicate greater confidence in the first value, and intuition would
suggest that a score with greater confidence should be placed above
scores of lesser confidence if the rating is the same
