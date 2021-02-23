#!/usr/bin/env ruby
# encoding: utf-8

require 'oj'
require 'unicode/emoji'

class String
  def safe_key_format
    safe_key_format = self.dup
    safe_key_format.gsub!(" ", "_")
    safe_key_format.gsub!("&", "and")
    safe_key_format.downcase!
    safe_key_format
  end
end

emoji_list = []
Unicode::Emoji.list.keys.each do |l|
    category_list = Hash.new
    category_list['category'] = l
    category_list['category_icon'] = ""
    category_list['emojis'] = Array.new
    Unicode::Emoji.list(l).keys.each do |subcategory|
        Unicode::Emoji.list(l, subcategory).each do |emoji|
            category_list['emojis'] << emoji
        end
    end

    if category_list['emojis'].count > 0
        category_list['category_icon'] = category_list['emojis'].first
        emoji_list << category_list
    end
end

File.open('./emoji.json', 'w+') do |f|
    f << Oj.dump(emoji_list)
end
